use std::collections::BTreeSet;
use std::error::Error;
use std::io;
use std::time::Duration;

use prost_types::Timestamp;
use vox_tinvest::account::{
    AccountCatalogue, AccountValues, BankAccountCatalogue, MarginAttributes, PortfolioState,
    PositionsState, ProviderTimestamp, UserInfo, UserTariff, WithdrawLimits,
};
use vox_tinvest::account_qualification::{
    AccountPurpose, Evidence, PreflightFailure, QualificationLedger, QualificationMode,
    classify_method_gate, classify_preflight, select_accounts, select_qualification_mode,
};
use vox_tinvest::generated::v1;
use vox_tinvest::operations::{
    OperationHistory, OperationsFilter, OperationsPaginator, PaginationFailure,
    PaginationFailureCause, canonical_legacy_operations,
};
use vox_tinvest::operations_stream::{
    OperationsStreamConfig, OperationsStreamError, OperationsStreamEvent,
    OperationsStreamSupervisor,
};
use vox_tinvest::reports::{BrokerReportState, ForeignIssuerReportState, ReportPageTraversal};
use vox_tinvest::{
    GrpcCredential, GrpcError, GrpcErrorKind, GrpcResponse, GrpcStreamError, SecretToken,
    TInvestGrpcClient,
};

fn failure(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(io::Error::other(message.into()))
}

fn optional_env(name: &str) -> Result<Option<String>, std::env::VarError> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error),
    }
}

fn qualified<T>(
    ledger: &mut QualificationLedger,
    method: &'static str,
    result: Result<GrpcResponse<T>, GrpcError>,
) -> Result<Option<T>, Box<dyn Error>> {
    match result {
        Ok(response) => {
            ledger.record(
                method,
                Evidence::Qualified("provider read + canonical decode".into()),
            )?;
            Ok(Some(response.body))
        }
        Err(error) => match &error.kind {
            GrpcErrorKind::Provider(provider) => {
                if let Some(gate) = classify_method_gate(method, provider) {
                    ledger.record(
                        method,
                        Evidence::GatedUnavailable(format!("{gate:?}; provider contract code")),
                    )?;
                    Ok(None)
                } else {
                    Err(Box::new(error))
                }
            }
            _ => Err(Box::new(error)),
        },
    }
}

fn recent_period() -> (Timestamp, Timestamp) {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    (
        Timestamp {
            seconds: now - 10 * 86_400,
            nanos: 0,
        },
        Timestamp {
            seconds: now - 3 * 86_400,
            nanos: 0,
        },
    )
}

fn record_deferred_rows(ledger: &mut QualificationLedger) -> Result<(), Box<dyn Error>> {
    for method in ["CurrencyTransfer", "PayIn"] {
        ledger.record(
            method,
            Evidence::GatedUnavailable("financial mutation; forbidden in issue #9".into()),
        )?;
    }
    for method in [
        "OpenSandboxAccount",
        "CloseSandboxAccount",
        "PostSandboxOrder",
        "PostSandboxOrderAsync",
        "ReplaceSandboxOrder",
        "CancelSandboxOrder",
        "SandboxPayIn",
        "PostSandboxStopOrder",
        "CancelSandboxStopOrder",
    ] {
        ledger.record(
            method,
            Evidence::GatedUnavailable("sandbox mutation; forbidden in issue #9".into()),
        )?;
    }
    for method in [
        "GetSandboxOrders",
        "GetSandboxOrderState",
        "GetSandboxOrderPrice",
        "GetSandboxMaxLots",
        "GetSandboxStopOrders",
    ] {
        ledger.record(
            method,
            Evidence::GatedUnavailable("execution read side owned by issue #10".into()),
        )?;
    }
    Ok(())
}

fn report_gate_or_error(
    method: &'static str,
    error: GrpcError,
) -> Result<Evidence, Box<dyn Error>> {
    if let GrpcErrorKind::Provider(provider) = &error.kind
        && let Some(gate) = classify_method_gate(method, provider)
    {
        return Ok(Evidence::GatedUnavailable(format!(
            "{gate:?}; provider contract code"
        )));
    }
    Err(Box::new(error))
}

fn task_pending(error: &GrpcError) -> bool {
    matches!(
        &error.kind,
        GrpcErrorKind::Provider(provider)
            if provider.code == tonic::Code::InvalidArgument
                && provider.has_provider_code("30058")
    )
}

fn pagination_result(
    method: &'static str,
    result: Result<OperationHistory, PaginationFailure>,
) -> Result<Evidence, Box<dyn Error>> {
    match result {
        Ok(history) => Ok(Evidence::Qualified(format!(
            "cursor exhausted; pages={} items={}",
            history.pages,
            history.items.len()
        ))),
        Err(failure) => match &failure.cause {
            PaginationFailureCause::Provider(GrpcError {
                kind: GrpcErrorKind::Provider(provider),
                ..
            }) => match classify_method_gate(method, provider) {
                Some(gate) => Ok(Evidence::GatedUnavailable(format!(
                    "{gate:?}; provider contract code"
                ))),
                None => Err(Box::new(failure)),
            },
            _ => Err(Box::new(failure)),
        },
    }
}

fn exact_successful_accounts(
    expected: &[String],
    actual: impl IntoIterator<Item = (String, i32)>,
) -> Result<Option<Evidence>, Box<dyn Error>> {
    let expected = expected.iter().cloned().collect::<BTreeSet<_>>();
    let mut observed = BTreeSet::new();
    let mut unavailable = false;
    for (account, status) in actual {
        if !matches!(status, 1 | 2) {
            return Err(failure(format!(
                "stream subscription rejected account={account} status={status}"
            )));
        }
        unavailable |= status == 2;
        if !observed.insert(account) {
            return Err(failure("stream subscription ACK repeated account"));
        }
    }
    if observed != expected {
        return Err(failure("stream subscription ACK account set mismatch"));
    }
    if unavailable {
        Ok(Some(Evidence::GatedUnavailable(
            "subscription status ACCOUNT_NOT_FOUND_OR_INSUFFICIENT_RIGHTS".into(),
        )))
    } else {
        Ok(None)
    }
}

fn stream_error(method: &'static str, error: GrpcStreamError) -> Result<Evidence, Box<dyn Error>> {
    if let GrpcStreamError::Provider(provider) = &error
        && let Some(gate) = classify_method_gate(method, provider)
    {
        return Ok(Evidence::GatedUnavailable(format!(
            "{gate:?}; provider contract code"
        )));
    }
    Err(Box::new(error))
}

async fn qualify_portfolio_stream(
    client: &TInvestGrpcClient,
    accounts: Vec<String>,
) -> Result<Evidence, Box<dyn Error>> {
    let mut stream = match client
        .open_portfolio_stream(v1::PortfolioStreamRequest {
            accounts: accounts.clone(),
            ping_settings: Some(v1::PingDelaySettings {
                ping_delay_ms: Some(5_000),
            }),
        })
        .await
    {
        Ok(stream) => stream,
        Err(error) => return report_gate_or_error("PortfolioStream", error),
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut subscribed = false;
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .ok_or_else(|| failure("PortfolioStream ACK+ping timeout"))?;
        let message = match tokio::time::timeout(remaining, stream.message()).await? {
            Ok(message) => message,
            Err(error) => return stream_error("PortfolioStream", error),
        };
        let Some(message) = message else {
            return Err(failure("PortfolioStream closed before ACK+ping"));
        };
        match message.payload {
            Some(v1::portfolio_stream_response::Payload::Subscriptions(result)) => {
                if let Some(gate) = exact_successful_accounts(
                    &accounts,
                    result
                        .accounts
                        .into_iter()
                        .map(|item| (item.account_id, item.subscription_status)),
                )? {
                    return Ok(gate);
                }
                subscribed = true;
            }
            Some(v1::portfolio_stream_response::Payload::Ping(_)) if subscribed => {
                return Ok(Evidence::Qualified("exact ACK + provider ping".into()));
            }
            _ => {}
        }
    }
}

async fn qualify_positions_stream(
    client: &TInvestGrpcClient,
    accounts: Vec<String>,
) -> Result<Evidence, Box<dyn Error>> {
    let mut stream = match client
        .open_positions_stream(v1::PositionsStreamRequest {
            accounts: accounts.clone(),
            with_initial_positions: true,
            ping_settings: Some(v1::PingDelaySettings {
                ping_delay_ms: Some(5_000),
            }),
        })
        .await
    {
        Ok(stream) => stream,
        Err(error) => return report_gate_or_error("PositionsStream", error),
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut subscribed = false;
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .ok_or_else(|| failure("PositionsStream ACK+ping timeout"))?;
        let message = match tokio::time::timeout(remaining, stream.message()).await? {
            Ok(message) => message,
            Err(error) => return stream_error("PositionsStream", error),
        };
        let Some(message) = message else {
            return Err(failure("PositionsStream closed before ACK+ping"));
        };
        match message.payload {
            Some(v1::positions_stream_response::Payload::Subscriptions(result)) => {
                if let Some(gate) = exact_successful_accounts(
                    &accounts,
                    result
                        .accounts
                        .into_iter()
                        .map(|item| (item.account_id, item.subscription_status)),
                )? {
                    return Ok(gate);
                }
                subscribed = true;
            }
            Some(v1::positions_stream_response::Payload::Ping(_)) if subscribed => {
                return Ok(Evidence::Qualified("exact ACK + provider ping".into()));
            }
            _ => {}
        }
    }
}

async fn qualify_broker_report(
    client: &TInvestGrpcClient,
    account_id: &str,
    from: Timestamp,
    to: Timestamp,
) -> Result<Evidence, Box<dyn Error>> {
    let generated = match client
        .get_broker_report(v1::BrokerReportRequest {
            payload: Some(
                v1::broker_report_request::Payload::GenerateBrokerReportRequest(
                    v1::GenerateBrokerReportRequest {
                        account_id: account_id.to_owned(),
                        from: Some(from),
                        to: Some(to),
                    },
                ),
            ),
        })
        .await
    {
        Ok(response) => BrokerReportState::try_from(response.body)?,
        Err(error) => return report_gate_or_error("GetBrokerReport", error),
    };
    let BrokerReportState::Generating { task_id } = generated else {
        return Err(failure("GetBrokerReport generation omitted task_id state"));
    };
    let mut pages = ReportPageTraversal::new(task_id)?;
    let mut pending_attempts = 0_u8;
    loop {
        let response = client
            .get_broker_report(v1::BrokerReportRequest {
                payload: Some(v1::broker_report_request::Payload::GetBrokerReportRequest(
                    v1::GetBrokerReportRequest {
                        task_id: pages.task_id().to_owned(),
                        page: Some(i32::try_from(pages.next_page())?),
                    },
                )),
            })
            .await;
        let body = match response {
            Ok(response) => response.body,
            Err(error) if task_pending(&error) && pending_attempts < 30 => {
                pending_attempts += 1;
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            Err(error) => return Err(Box::new(error)),
        };
        let BrokerReportState::Ready(page) = BrokerReportState::try_from(body)? else {
            return Err(failure(
                "GetBrokerReport readback returned generation payload",
            ));
        };
        if page.task_id.as_deref() != Some(pages.task_id()) {
            return Err(failure("GetBrokerReport readback task_id mismatch"));
        }
        if pages.observe(page.page, page.pages_count)? {
            return Ok(Evidence::Qualified(format!(
                "generate + task readback + pages 0..={}",
                page.page
            )));
        }
    }
}

async fn qualify_foreign_report(
    client: &TInvestGrpcClient,
    account_id: &str,
    from: Timestamp,
    to: Timestamp,
) -> Result<Evidence, Box<dyn Error>> {
    let generated = match client
        .get_dividends_foreign_issuer(v1::GetDividendsForeignIssuerRequest {
            payload: Some(
                v1::get_dividends_foreign_issuer_request::Payload::GenerateDivForeignIssuerReport(
                    v1::GenerateDividendsForeignIssuerReportRequest {
                        account_id: account_id.to_owned(),
                        from: Some(from),
                        to: Some(to),
                    },
                ),
            ),
        })
        .await
    {
        Ok(response) => ForeignIssuerReportState::try_from(response.body)?,
        Err(error) => return report_gate_or_error("GetDividendsForeignIssuer", error),
    };
    let ForeignIssuerReportState::Generating { task_id } = generated else {
        return Err(failure(
            "GetDividendsForeignIssuer generation omitted task_id state",
        ));
    };
    let mut pages = ReportPageTraversal::new(task_id)?;
    let mut pending_attempts = 0_u8;
    loop {
        let response = client
            .get_dividends_foreign_issuer(v1::GetDividendsForeignIssuerRequest {
                payload: Some(
                    v1::get_dividends_foreign_issuer_request::Payload::GetDivForeignIssuerReport(
                        v1::GetDividendsForeignIssuerReportRequest {
                            task_id: pages.task_id().to_owned(),
                            page: Some(i32::try_from(pages.next_page())?),
                        },
                    ),
                ),
            })
            .await;
        let body = match response {
            Ok(response) => response.body,
            Err(error) if task_pending(&error) && pending_attempts < 30 => {
                pending_attempts += 1;
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            Err(error) => return Err(Box::new(error)),
        };
        let ForeignIssuerReportState::Ready(page) = ForeignIssuerReportState::try_from(body)?
        else {
            return Err(failure(
                "GetDividendsForeignIssuer readback returned generation payload",
            ));
        };
        if pages.observe(page.page, page.pages_count)? {
            return Ok(Evidence::Qualified(format!(
                "generate + task readback + pages 0..={}",
                page.page
            )));
        }
    }
}

#[test]
fn generated_stream_ack_requires_exact_eligible_account_set() {
    let expected = vec!["broker-a".to_owned(), "broker-b".to_owned()];
    assert_eq!(
        exact_successful_accounts(
            &expected,
            [("broker-b".to_owned(), 1), ("broker-a".to_owned(), 1)]
        )
        .expect("exact ACK"),
        None
    );
    assert!(exact_successful_accounts(&expected, [("broker-a".to_owned(), 1)]).is_err());
    assert!(matches!(
        exact_successful_accounts(
            &expected,
            [("broker-a".to_owned(), 1), ("broker-b".to_owned(), 2)]
        ),
        Ok(Some(Evidence::GatedUnavailable(_)))
    ));
}

#[tokio::test]
#[ignore = "requires TINVEST_SANDBOX_TOKEN or TINVEST_TOKEN; one read-only qualification"]
async fn current_account_read_side_qualifies_over_grpc() -> Result<(), Box<dyn Error>> {
    let production_token = optional_env("TINVEST_TOKEN")?;
    let sandbox_token = optional_env("TINVEST_SANDBOX_TOKEN")?;
    let explicit_mode = optional_env("TINVEST_QUALIFICATION_ENV")?;
    let mode = select_qualification_mode(
        explicit_mode.as_deref(),
        production_token.is_some(),
        sandbox_token.is_some(),
    )?;
    let client = match mode {
        QualificationMode::SandboxOnly => {
            TInvestGrpcClient::sandbox(GrpcCredential::Sandbox(SecretToken::new(
                sandbox_token
                    .as_ref()
                    .ok_or_else(|| failure("SANDBOX_ONLY token selection invariant"))?
                    .clone(),
            )?))?
        }
        QualificationMode::ProductionReadOnly => {
            TInvestGrpcClient::production(GrpcCredential::Production(SecretToken::new(
                production_token
                    .as_ref()
                    .ok_or_else(|| failure("PRODUCTION_READ_ONLY token selection invariant"))?
                    .clone(),
            )?))?
        }
    };
    let mut ledger = QualificationLedger::default();

    let accounts = match client
        .get_accounts(v1::GetAccountsRequest {
            status: Some(v1::AccountStatus::Open as i32),
        })
        .await
    {
        Ok(response) => {
            ledger.record(
                "GetAccounts",
                Evidence::Qualified(format!(
                    "{} credential preflight; OPEN accounts",
                    mode.wire_name()
                )),
            )?;
            AccountCatalogue::try_from(response.body)?
        }
        Err(error) => match classify_preflight(&error) {
            PreflightFailure::CredentialInvalidOrInactive => {
                return Err(failure(format!(
                    "CREDENTIAL_INVALID_OR_INACTIVE environment={} provider_code=40003",
                    mode.wire_name()
                )));
            }
            PreflightFailure::InsufficientPermission => {
                return Err(failure(format!(
                    "CREDENTIAL_INSUFFICIENT_PERMISSION environment={} provider_code=40002",
                    mode.wire_name()
                )));
            }
            PreflightFailure::Other => return Err(Box::new(error)),
        },
    };

    let general = select_accounts(&accounts.accounts, AccountPurpose::GeneralRead);
    let margin = select_accounts(&accounts.accounts, AccountPurpose::Margin);
    let reports = select_accounts(&accounts.accounts, AccountPurpose::Report);
    let stream_accounts = select_accounts(&accounts.accounts, AccountPurpose::OperationsStream);
    let general_ids = general
        .selected
        .iter()
        .map(|account| account.account_id.clone())
        .collect::<Vec<_>>();
    let general_id = general_ids.first().cloned();

    if let Some(account) = margin.selected.first() {
        if let Some(body) = qualified(
            &mut ledger,
            "GetMarginAttributes",
            client
                .get_margin_attributes(v1::GetMarginAttributesRequest {
                    account_id: account.account_id.clone(),
                })
                .await,
        )? {
            let _ = MarginAttributes::try_from(body)?;
        }
    } else {
        ledger.record(
            "GetMarginAttributes",
            Evidence::GatedUnavailable("no open readable brokerage/IIS account".into()),
        )?;
    }
    if let Some(body) = qualified(
        &mut ledger,
        "GetUserTariff",
        client.get_user_tariff(v1::GetUserTariffRequest {}).await,
    )? {
        let _ = UserTariff::from(body);
    }
    if let Some(body) = qualified(
        &mut ledger,
        "GetInfo",
        client.get_info(v1::GetInfoRequest {}).await,
    )? {
        let _ = UserInfo::from(body);
    }
    if let Some(body) = qualified(
        &mut ledger,
        "GetBankAccounts",
        client
            .get_bank_accounts(v1::GetBankAccountsRequest {})
            .await,
    )? {
        let _ = BankAccountCatalogue::try_from(body)?;
    }
    if general_ids.is_empty() {
        ledger.record(
            "GetAccountValues",
            Evidence::GatedUnavailable("no open readable investment account".into()),
        )?;
    } else if let Some(body) = qualified(
        &mut ledger,
        "GetAccountValues",
        client
            .get_account_values(v1::GetAccountValuesRequest {
                accounts: general_ids.clone(),
                values: vec![
                    v1::AccountValue::MarginFee as i32,
                    v1::AccountValue::AmountWithoutExtraFee as i32,
                ],
            })
            .await,
    )? {
        let _ = AccountValues::try_from(body)?;
    }

    let (from, to) = recent_period();
    if let Some(account_id) = &general_id {
        if let Some(body) = qualified(
            &mut ledger,
            "GetOperations",
            client
                .get_operations(v1::OperationsRequest {
                    account_id: account_id.clone(),
                    from: Some(from),
                    to: Some(to),
                    state: None,
                    figi: None,
                })
                .await,
        )? {
            let _ = canonical_legacy_operations(body)?;
        }
        if let Some(body) = qualified(
            &mut ledger,
            "GetPortfolio",
            client
                .get_portfolio(v1::PortfolioRequest {
                    account_id: account_id.clone(),
                    currency: None,
                })
                .await,
        )? {
            let _ = PortfolioState::try_from(body)?;
        }
        if let Some(body) = qualified(
            &mut ledger,
            "GetPositions",
            client
                .get_positions(v1::PositionsRequest {
                    account_id: account_id.clone(),
                })
                .await,
        )? {
            let _ = PositionsState::try_from(body)?;
        }
        if let Some(body) = qualified(
            &mut ledger,
            "GetWithdrawLimits",
            client
                .get_withdraw_limits(v1::WithdrawLimitsRequest {
                    account_id: account_id.clone(),
                })
                .await,
        )? {
            let _ = WithdrawLimits::try_from(body)?;
        }
        let history = OperationsPaginator::new(OperationsFilter {
            account_id: account_id.clone(),
            instrument_id: None,
            from: Some(ProviderTimestamp::try_from(from)?),
            to: Some(ProviderTimestamp::try_from(to)?),
            initial_cursor: None,
            page_size: Some(100),
            operation_types: Vec::new(),
            state: None,
            without_commissions: None,
            without_trades: None,
            without_overnights: None,
        })?
        .collect(&client)
        .await;
        ledger.record(
            "GetOperationsByCursor",
            pagination_result("GetOperationsByCursor", history)?,
        )?;
    } else {
        for method in [
            "GetOperations",
            "GetPortfolio",
            "GetPositions",
            "GetWithdrawLimits",
            "GetOperationsByCursor",
        ] {
            ledger.record(
                method,
                Evidence::GatedUnavailable("no open readable investment account".into()),
            )?;
        }
    }

    if mode == QualificationMode::SandboxOnly {
        for method in ["GetBrokerReport", "GetDividendsForeignIssuer"] {
            ledger.record(
                method,
                Evidence::GatedUnavailable(
                    "reason=ENVIRONMENT_UNSUPPORTED_SANDBOX; official environment matrix".into(),
                ),
            )?;
        }
    } else if let Some(account) = reports.selected.first() {
        ledger.record(
            "GetBrokerReport",
            qualify_broker_report(&client, &account.account_id, from, to).await?,
        )?;
        let from_time = time::OffsetDateTime::from_unix_timestamp(from.seconds)?;
        let year_start = time::Date::from_calendar_date(from_time.year(), time::Month::January, 1)?
            .with_hms(0, 0, 0)?
            .assume_utc()
            .unix_timestamp();
        ledger.record(
            "GetDividendsForeignIssuer",
            qualify_foreign_report(
                &client,
                &account.account_id,
                Timestamp {
                    seconds: from.seconds.max(year_start),
                    nanos: 0,
                },
                to,
            )
            .await?,
        )?;
    } else {
        for method in ["GetBrokerReport", "GetDividendsForeignIssuer"] {
            ledger.record(
                method,
                Evidence::GatedUnavailable("no open readable brokerage/IIS account".into()),
            )?;
        }
    }

    let stream_ids = stream_accounts
        .selected
        .iter()
        .map(|account| account.account_id.clone())
        .collect::<Vec<_>>();
    if stream_ids.is_empty() {
        for method in ["PortfolioStream", "PositionsStream", "OperationsStream"] {
            ledger.record(
                method,
                Evidence::GatedUnavailable("no open readable investment account".into()),
            )?;
        }
    } else {
        ledger.record(
            "PortfolioStream",
            qualify_portfolio_stream(&client, stream_ids.clone()).await?,
        )?;
        ledger.record(
            "PositionsStream",
            qualify_positions_stream(&client, stream_ids.clone()).await?,
        )?;
        let supervisor = OperationsStreamSupervisor::new(
            client.clone(),
            OperationsStreamConfig {
                ping_delay_ms: 5_000,
                stale_timeout: Duration::from_secs(20),
                ..Default::default()
            },
        )?;
        let mut stream = supervisor.start(stream_ids.clone())?;
        let mut subscribed = false;
        loop {
            let event = tokio::time::timeout(Duration::from_secs(30), stream.recv())
                .await?
                .ok_or_else(|| failure("operations stream event channel closed"))?;
            match event {
                OperationsStreamEvent::Subscribed { accounts, .. } => {
                    if accounts != stream_ids {
                        return Err(failure("OperationsStream ACK account set mismatch"));
                    }
                    subscribed = true;
                }
                OperationsStreamEvent::Ping { .. } if subscribed => {
                    ledger.record(
                        "OperationsStream",
                        Evidence::Qualified(
                            "eligible account set + exact ACK + provider ping + reconnect state"
                                .into(),
                        ),
                    )?;
                    stream.stop();
                    break;
                }
                OperationsStreamEvent::Fault(OperationsStreamError::SubscriptionRejected {
                    status: 2,
                }) => {
                    ledger.record(
                        "OperationsStream",
                        Evidence::GatedUnavailable(
                            "subscription status ACCOUNT_NOT_FOUND_OR_INSUFFICIENT_RIGHTS".into(),
                        ),
                    )?;
                    stream.stop();
                    break;
                }
                OperationsStreamEvent::Fault(OperationsStreamError::Connect(error))
                    if classify_preflight(&error)
                        == PreflightFailure::CredentialInvalidOrInactive =>
                {
                    return Err(failure(format!(
                        "CREDENTIAL_INVALID_OR_INACTIVE environment={} provider_code=40003 stream=OperationsStream",
                        mode.wire_name()
                    )));
                }
                OperationsStreamEvent::Fault(error) => return Err(Box::new(error)),
                _ => {}
            }
        }
    }

    let sandbox_client = match mode {
        QualificationMode::SandboxOnly => Some(client.clone()),
        QualificationMode::ProductionReadOnly => sandbox_token
            .map(SecretToken::new)
            .transpose()?
            .map(|token| TInvestGrpcClient::sandbox(GrpcCredential::Sandbox(token)))
            .transpose()?,
    };
    match sandbox_client {
        None => {
            for method in [
                "GetSandboxAccounts",
                "GetSandboxPositions",
                "GetSandboxOperations",
                "GetSandboxOperationsByCursor",
                "GetSandboxPortfolio",
                "GetSandboxWithdrawLimits",
            ] {
                ledger.record(
                    method,
                    Evidence::GatedUnavailable(
                        "SANDBOX_ENVIRONMENT: TINVEST_SANDBOX_TOKEN not configured".into(),
                    ),
                )?;
            }
        }
        Some(sandbox) => {
            let sandbox_accounts = match sandbox
                .get_sandbox_accounts(v1::GetAccountsRequest {
                    status: Some(v1::AccountStatus::Open as i32),
                })
                .await
            {
                Ok(response) => {
                    ledger.record(
                        "GetSandboxAccounts",
                        Evidence::Qualified("sandbox credential preflight; OPEN accounts".into()),
                    )?;
                    AccountCatalogue::try_from(response.body)?
                }
                Err(error) => match classify_preflight(&error) {
                    PreflightFailure::CredentialInvalidOrInactive => {
                        return Err(failure(
                            "CREDENTIAL_INVALID_OR_INACTIVE environment=SANDBOX provider_code=40003",
                        ));
                    }
                    PreflightFailure::InsufficientPermission => {
                        return Err(failure(
                            "CREDENTIAL_INSUFFICIENT_PERMISSION environment=SANDBOX provider_code=40002",
                        ));
                    }
                    PreflightFailure::Other => return Err(Box::new(error)),
                },
            };
            let selected = select_accounts(&sandbox_accounts.accounts, AccountPurpose::GeneralRead);
            if let Some(account) = selected.selected.first() {
                let account_id = account.account_id.clone();
                if let Some(body) = qualified(
                    &mut ledger,
                    "GetSandboxPortfolio",
                    sandbox
                        .get_sandbox_portfolio(v1::PortfolioRequest {
                            account_id: account_id.clone(),
                            currency: None,
                        })
                        .await,
                )? {
                    let _ = PortfolioState::try_from(body)?;
                }
                if let Some(body) = qualified(
                    &mut ledger,
                    "GetSandboxPositions",
                    sandbox
                        .get_sandbox_positions(v1::PositionsRequest {
                            account_id: account_id.clone(),
                        })
                        .await,
                )? {
                    let _ = PositionsState::try_from(body)?;
                }
                if let Some(body) = qualified(
                    &mut ledger,
                    "GetSandboxWithdrawLimits",
                    sandbox
                        .get_sandbox_withdraw_limits(v1::WithdrawLimitsRequest {
                            account_id: account_id.clone(),
                        })
                        .await,
                )? {
                    let _ = WithdrawLimits::try_from(body)?;
                }
                if let Some(body) = qualified(
                    &mut ledger,
                    "GetSandboxOperations",
                    sandbox
                        .get_sandbox_operations(v1::OperationsRequest {
                            account_id: account_id.clone(),
                            from: None,
                            to: None,
                            state: None,
                            figi: None,
                        })
                        .await,
                )? {
                    let _ = canonical_legacy_operations(body)?;
                }
                let history = OperationsPaginator::new(OperationsFilter {
                    account_id,
                    instrument_id: None,
                    from: None,
                    to: None,
                    initial_cursor: None,
                    page_size: Some(100),
                    operation_types: Vec::new(),
                    state: None,
                    without_commissions: None,
                    without_trades: None,
                    without_overnights: None,
                })?
                .collect_sandbox(&sandbox)
                .await;
                ledger.record(
                    "GetSandboxOperationsByCursor",
                    pagination_result("GetSandboxOperationsByCursor", history)?,
                )?;
            } else {
                for method in [
                    "GetSandboxPositions",
                    "GetSandboxOperations",
                    "GetSandboxOperationsByCursor",
                    "GetSandboxPortfolio",
                    "GetSandboxWithdrawLimits",
                ] {
                    ledger.record(
                        method,
                        Evidence::GatedUnavailable(
                            "authenticated sandbox has no open readable account".into(),
                        ),
                    )?;
                }
            }
        }
    }

    record_deferred_rows(&mut ledger)?;
    for (method, evidence) in ledger.finish()? {
        match evidence {
            Evidence::Qualified(detail) => println!("QUALIFIED {method}: {detail}"),
            Evidence::GatedUnavailable(detail) => {
                println!("GATED/UNAVAILABLE {method}: {detail}")
            }
        }
    }
    Ok(())
}

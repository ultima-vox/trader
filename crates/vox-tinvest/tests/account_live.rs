use std::collections::BTreeSet;
use std::error::Error;
use std::io;
use std::time::Duration;

use prost::Message;
use prost_types::Timestamp;
use vox_tinvest::account::{
    AccountCatalogue, AccountValues, BankAccountCatalogue, MarginAttributes, PortfolioState,
    PositionsState, ProviderTimestamp, UserInfo, UserTariff, WithdrawLimits,
};
use vox_tinvest::account_qualification::{
    AccountPurpose, Evidence, PreflightFailure, QualificationLedger, QualificationMode,
    QualificationSummary, adapter_failure, classify_method_gate, classify_preflight, grpc_failure,
    persistent_sandbox_provider_limitation, select_accounts, select_qualification_mode,
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

fn qualified<T, D, E, Decode>(
    ledger: &mut QualificationLedger,
    method: &'static str,
    result: Result<GrpcResponse<T>, GrpcError>,
    decode: Decode,
) -> Result<Option<D>, Box<dyn Error>>
where
    E: std::fmt::Display,
    Decode: FnOnce(T) -> Result<D, E>,
{
    qualified_with_context(ledger, method, result, decode, None)
}

struct LiveFailureContext<'a> {
    environment: QualificationMode,
    request_shape: String,
    sensitive_values: &'a [String],
    sibling_users_reads_qualified: bool,
}

fn qualified_with_context<T, D, E, Decode>(
    ledger: &mut QualificationLedger,
    method: &'static str,
    result: Result<GrpcResponse<T>, GrpcError>,
    decode: Decode,
    context: Option<LiveFailureContext<'_>>,
) -> Result<Option<D>, Box<dyn Error>>
where
    E: std::fmt::Display,
    Decode: FnOnce(T) -> Result<D, E>,
{
    match result {
        Ok(response) => match decode(response.body) {
            Ok(decoded) => {
                ledger.record(
                    method,
                    Evidence::Qualified("provider read + canonical decode".into()),
                )?;
                Ok(Some(decoded))
            }
            Err(error) => {
                let mut failure = adapter_failure(method, error.to_string());
                if let Some(context) = &context {
                    failure = failure.with_live_context(
                        context.environment.wire_name(),
                        context.request_shape.clone(),
                        context.sensitive_values,
                    );
                }
                ledger.record(method, Evidence::Failed(failure))?;
                Ok(None)
            }
        },
        Err(error) => match &error.kind {
            GrpcErrorKind::Provider(provider) => {
                if context.as_ref().is_some_and(|context| {
                    context.environment == QualificationMode::SandboxOnly
                        && context.sibling_users_reads_qualified
                        && persistent_sandbox_provider_limitation(method, &error)
                }) {
                    let failure = grpc_failure(method, &error);
                    ledger.record(
                        method,
                        Evidence::GatedUnavailable(format!(
                            "EXTERNAL_PROVIDER_LIMITATION_SANDBOX; advertised exact generated request reproducibly returned grpc_status={:?} provider_code={} provider_message={} attempt={} tracking_id={} environment={} request_shape=GetBankAccountsRequest {{}}",
                            failure.grpc_status,
                            failure.provider_code.as_deref().unwrap_or("-"),
                            failure.provider_message.as_deref().unwrap_or("-"),
                            failure.attempt.unwrap_or_default(),
                            failure.tracking_id.as_deref().unwrap_or("-"),
                            context
                                .as_ref()
                                .map_or("-", |context| context.environment.wire_name())
                        )),
                    )?;
                    return Ok(None);
                }
                if let Some(gate) = classify_method_gate(method, provider) {
                    ledger.record(
                        method,
                        Evidence::GatedUnavailable(format!("{gate:?}; provider contract code")),
                    )?;
                    Ok(None)
                } else {
                    let mut failure = grpc_failure(method, &error);
                    if let Some(context) = &context {
                        failure = failure.with_live_context(
                            context.environment.wire_name(),
                            context.request_shape.clone(),
                            context.sensitive_values,
                        );
                    }
                    ledger.record(method, Evidence::Failed(failure))?;
                    Ok(None)
                }
            }
            _ => {
                let mut failure = grpc_failure(method, &error);
                if let Some(context) = &context {
                    failure = failure.with_live_context(
                        context.environment.wire_name(),
                        context.request_shape.clone(),
                        context.sensitive_values,
                    );
                }
                ledger.record(method, Evidence::Failed(failure))?;
                Ok(None)
            }
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

fn account_value_probe_requests(account_ids: &[String]) -> Vec<v1::GetAccountValuesRequest> {
    let Some(account_id) = account_ids.first() else {
        return Vec::new();
    };
    [
        v1::AccountValue::MarginFee,
        v1::AccountValue::AmountWithoutExtraFee,
    ]
    .into_iter()
    .map(|value| v1::GetAccountValuesRequest {
        accounts: vec![account_id.clone()],
        values: vec![value as i32],
    })
    .collect()
}

async fn qualify_account_values(
    client: &TInvestGrpcClient,
    mode: QualificationMode,
    account_ids: &[String],
) -> Evidence {
    let requests = account_value_probe_requests(account_ids);
    for request in requests {
        let requested_value = request.values[0];
        let request_shape = format!(
            "GetAccountValuesRequest {{ accounts: 1 OPEN readable brokerage/IIS account, values: [{requested_value}] }}"
        );
        match client.get_account_values(request).await {
            Ok(response) => {
                let decoded = match AccountValues::try_from(response.body) {
                    Ok(decoded) => decoded,
                    Err(error) => {
                        return Evidence::Failed(
                            adapter_failure("GetAccountValues", error.to_string())
                                .with_live_context(mode.wire_name(), request_shape, account_ids),
                        );
                    }
                };
                if decoded.accounts.iter().any(|account| {
                    account.account_id != account_ids[0]
                        || account
                            .values
                            .iter()
                            .any(|value| value.name != requested_value)
                }) {
                    return Evidence::Failed(
                        adapter_failure(
                            "GetAccountValues",
                            "response identity/value differs from isolated request",
                        )
                        .with_live_context(
                            mode.wire_name(),
                            request_shape,
                            account_ids,
                        ),
                    );
                }
            }
            Err(error) => {
                if let GrpcErrorKind::Provider(provider) = &error.kind
                    && let Some(gate) = classify_method_gate("GetAccountValues", provider)
                {
                    return Evidence::GatedUnavailable(format!("{gate:?}; provider contract code"));
                }
                return Evidence::Failed(
                    grpc_failure("GetAccountValues", &error).with_live_context(
                        mode.wire_name(),
                        request_shape,
                        account_ids,
                    ),
                );
            }
        }
    }
    Evidence::Qualified(
        "isolated account/value probes; MARGIN_FEE + AMOUNT_WITHOUT_EXTRA_FEE canonical decode"
            .into(),
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

fn grpc_observation(method: &'static str, error: GrpcError) -> Evidence {
    if let GrpcErrorKind::Provider(provider) = &error.kind
        && let Some(gate) = classify_method_gate(method, provider)
    {
        return Evidence::GatedUnavailable(format!("{gate:?}; provider contract code"));
    }
    Evidence::Failed(grpc_failure(method, &error))
}

fn aggregate_result(method: &'static str, result: Result<Evidence, Box<dyn Error>>) -> Evidence {
    result.unwrap_or_else(|error| Evidence::Failed(adapter_failure(method, error.to_string())))
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
            PaginationFailureCause::Provider(
                error @ GrpcError {
                    kind: GrpcErrorKind::Provider(provider),
                    ..
                },
            ) => match classify_method_gate(method, provider) {
                Some(gate) => Ok(Evidence::GatedUnavailable(format!(
                    "{gate:?}; provider contract code"
                ))),
                None => Ok(Evidence::Failed(grpc_failure(method, error))),
            },
            _ => Ok(Evidence::Failed(adapter_failure(
                method,
                failure.to_string(),
            ))),
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
    Ok(Evidence::Failed(adapter_failure(method, error.to_string())))
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
        Err(error) => return Ok(grpc_observation("PortfolioStream", error)),
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
        Err(error) => return Ok(grpc_observation("PositionsStream", error)),
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
        Err(error) => return Ok(grpc_observation("GetBrokerReport", error)),
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
            Err(error) => return Ok(grpc_observation("GetBrokerReport", error)),
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
        Err(error) => return Ok(grpc_observation("GetDividendsForeignIssuer", error)),
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
            Err(error) => {
                return Ok(grpc_observation("GetDividendsForeignIssuer", error));
            }
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

async fn qualify_operations_stream(
    client: &TInvestGrpcClient,
    accounts: Vec<String>,
) -> Result<Evidence, Box<dyn Error>> {
    let supervisor = OperationsStreamSupervisor::new(
        client.clone(),
        OperationsStreamConfig {
            ping_delay_ms: 5_000,
            stale_timeout: Duration::from_secs(20),
            ..Default::default()
        },
    )?;
    let mut stream = supervisor.start(accounts.clone())?;
    let mut subscribed = false;
    let result = loop {
        let event = tokio::time::timeout(Duration::from_secs(30), stream.recv())
            .await?
            .ok_or_else(|| failure("operations stream event channel closed"))?;
        match event {
            OperationsStreamEvent::Subscribed {
                accounts: observed, ..
            } => {
                if observed != accounts {
                    break Err(failure("OperationsStream ACK account set mismatch"));
                }
                subscribed = true;
            }
            OperationsStreamEvent::Ping { .. } if subscribed => {
                break Ok(Evidence::Qualified(
                    "eligible account set + exact ACK + provider ping + reconnect state".into(),
                ));
            }
            OperationsStreamEvent::Fault(OperationsStreamError::SubscriptionRejected {
                status: 2,
            }) => {
                break Ok(Evidence::GatedUnavailable(
                    "subscription status ACCOUNT_NOT_FOUND_OR_INSUFFICIENT_RIGHTS".into(),
                ));
            }
            OperationsStreamEvent::Fault(OperationsStreamError::Connect(error)) => {
                break Ok(grpc_observation("OperationsStream", error));
            }
            OperationsStreamEvent::Fault(error) => break Err(Box::new(error)),
            _ => {}
        }
    };
    stream.stop();
    result
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

#[test]
fn users_service_blocker_requests_are_minimal_and_generated() {
    assert_eq!(v1::GetBankAccountsRequest {}.encoded_len(), 0);

    let requests = account_value_probe_requests(&["eligible-a".into(), "eligible-b".into()]);
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.accounts == ["eligible-a"])
    );
    assert_eq!(
        requests
            .iter()
            .map(|request| request.values.as_slice())
            .collect::<Vec<_>>(),
        [
            [v1::AccountValue::MarginFee as i32].as_slice(),
            [v1::AccountValue::AmountWithoutExtraFee as i32].as_slice(),
        ]
    );
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
        Ok(response) => match AccountCatalogue::try_from(response.body) {
            Ok(accounts) => {
                ledger.record(
                    "GetAccounts",
                    Evidence::Qualified(format!(
                        "{} credential preflight; OPEN accounts",
                        mode.wire_name()
                    )),
                )?;
                Some(accounts)
            }
            Err(error) => {
                ledger.record(
                    "GetAccounts",
                    Evidence::Failed(adapter_failure("GetAccounts", error.to_string())),
                )?;
                None
            }
        },
        Err(error) => match classify_preflight(&error) {
            PreflightFailure::CredentialInvalidOrInactive => {
                return Err(failure(format!(
                    "CREDENTIAL_INVALID_OR_INACTIVE environment={} provider_code=40003",
                    mode.wire_name()
                )));
            }
            PreflightFailure::InsufficientPermission | PreflightFailure::Other => {
                ledger.record(
                    "GetAccounts",
                    Evidence::Failed(grpc_failure("GetAccounts", &error)),
                )?;
                None
            }
        },
    };

    let account_discovery_failed = accounts.is_none();
    let account_rows = accounts
        .as_ref()
        .map_or(&[][..], |catalogue| catalogue.accounts.as_slice());
    let general = select_accounts(account_rows, AccountPurpose::GeneralRead);
    let account_values = select_accounts(account_rows, AccountPurpose::AccountValues);
    let margin = select_accounts(account_rows, AccountPurpose::Margin);
    let reports = select_accounts(account_rows, AccountPurpose::Report);
    let stream_accounts = select_accounts(account_rows, AccountPurpose::OperationsStream);
    let general_ids = general
        .selected
        .iter()
        .map(|account| account.account_id.clone())
        .collect::<Vec<_>>();
    let account_value_ids = account_values
        .selected
        .iter()
        .map(|account| account.account_id.clone())
        .collect::<Vec<_>>();
    let general_id = general_ids.first().cloned();

    if let Some(account) = margin.selected.first() {
        let _ = qualified(
            &mut ledger,
            "GetMarginAttributes",
            client
                .get_margin_attributes(v1::GetMarginAttributesRequest {
                    account_id: account.account_id.clone(),
                })
                .await,
            MarginAttributes::try_from,
        )?;
    } else if account_discovery_failed {
        ledger.record(
            "GetMarginAttributes",
            Evidence::BlockedByPrerequisite("GetAccounts failed".into()),
        )?;
    } else {
        ledger.record(
            "GetMarginAttributes",
            Evidence::GatedUnavailable("no open readable brokerage/IIS account".into()),
        )?;
    }
    let tariff = qualified(
        &mut ledger,
        "GetUserTariff",
        client.get_user_tariff(v1::GetUserTariffRequest {}).await,
        |body| Ok::<_, std::convert::Infallible>(UserTariff::from(body)),
    )?;
    let info = qualified(
        &mut ledger,
        "GetInfo",
        client.get_info(v1::GetInfoRequest {}).await,
        |body| Ok::<_, std::convert::Infallible>(UserInfo::from(body)),
    )?;
    let _ = qualified_with_context(
        &mut ledger,
        "GetBankAccounts",
        client
            .get_bank_accounts(v1::GetBankAccountsRequest {})
            .await,
        BankAccountCatalogue::try_from,
        Some(LiveFailureContext {
            environment: mode,
            request_shape: "GetBankAccountsRequest {}".into(),
            sensitive_values: &[],
            sibling_users_reads_qualified: !account_discovery_failed
                && tariff.is_some()
                && info.is_some(),
        }),
    )?;
    if account_discovery_failed {
        ledger.record(
            "GetAccountValues",
            Evidence::BlockedByPrerequisite("GetAccounts failed".into()),
        )?;
    } else if mode == QualificationMode::SandboxOnly {
        ledger.record(
            "GetAccountValues",
            Evidence::GatedUnavailable(
                "ENVIRONMENT_DATA_UNAVAILABLE_SANDBOX; official sandbox contract states additional account indicators are not calculated"
                    .into(),
            ),
        )?;
    } else if account_value_ids.is_empty() {
        ledger.record(
            "GetAccountValues",
            Evidence::GatedUnavailable(
                "no OPEN readable brokerage/IIS account valid for GetAccountValues".into(),
            ),
        )?;
    } else {
        ledger.record(
            "GetAccountValues",
            qualify_account_values(&client, mode, &account_value_ids).await,
        )?;
    }

    let (from, to) = recent_period();
    if let Some(account_id) = &general_id {
        let _ = qualified(
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
            canonical_legacy_operations,
        )?;
        let _ = qualified(
            &mut ledger,
            "GetPortfolio",
            client
                .get_portfolio(v1::PortfolioRequest {
                    account_id: account_id.clone(),
                    currency: None,
                })
                .await,
            PortfolioState::try_from,
        )?;
        let _ = qualified(
            &mut ledger,
            "GetPositions",
            client
                .get_positions(v1::PositionsRequest {
                    account_id: account_id.clone(),
                })
                .await,
            PositionsState::try_from,
        )?;
        let _ = qualified(
            &mut ledger,
            "GetWithdrawLimits",
            client
                .get_withdraw_limits(v1::WithdrawLimitsRequest {
                    account_id: account_id.clone(),
                })
                .await,
            WithdrawLimits::try_from,
        )?;
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
    } else if account_discovery_failed {
        for method in [
            "GetOperations",
            "GetPortfolio",
            "GetPositions",
            "GetWithdrawLimits",
            "GetOperationsByCursor",
        ] {
            ledger.record(
                method,
                Evidence::BlockedByPrerequisite("GetAccounts failed".into()),
            )?;
        }
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
    } else if account_discovery_failed {
        for method in ["GetBrokerReport", "GetDividendsForeignIssuer"] {
            ledger.record(
                method,
                Evidence::BlockedByPrerequisite("GetAccounts failed".into()),
            )?;
        }
    } else if let Some(account) = reports.selected.first() {
        ledger.record(
            "GetBrokerReport",
            aggregate_result(
                "GetBrokerReport",
                qualify_broker_report(&client, &account.account_id, from, to).await,
            ),
        )?;
        let from_time = time::OffsetDateTime::from_unix_timestamp(from.seconds)?;
        let year_start = time::Date::from_calendar_date(from_time.year(), time::Month::January, 1)?
            .with_hms(0, 0, 0)?
            .assume_utc()
            .unix_timestamp();
        ledger.record(
            "GetDividendsForeignIssuer",
            aggregate_result(
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
                .await,
            ),
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
    if account_discovery_failed {
        for method in ["PortfolioStream", "PositionsStream", "OperationsStream"] {
            ledger.record(
                method,
                Evidence::BlockedByPrerequisite("GetAccounts failed".into()),
            )?;
        }
    } else if stream_ids.is_empty() {
        for method in ["PortfolioStream", "PositionsStream", "OperationsStream"] {
            ledger.record(
                method,
                Evidence::GatedUnavailable("no open readable investment account".into()),
            )?;
        }
    } else {
        ledger.record(
            "PortfolioStream",
            aggregate_result(
                "PortfolioStream",
                qualify_portfolio_stream(&client, stream_ids.clone()).await,
            ),
        )?;
        ledger.record(
            "PositionsStream",
            aggregate_result(
                "PositionsStream",
                qualify_positions_stream(&client, stream_ids.clone()).await,
            ),
        )?;
        ledger.record(
            "OperationsStream",
            aggregate_result(
                "OperationsStream",
                qualify_operations_stream(&client, stream_ids.clone()).await,
            ),
        )?;
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
                Ok(response) => match AccountCatalogue::try_from(response.body) {
                    Ok(accounts) => {
                        ledger.record(
                            "GetSandboxAccounts",
                            Evidence::Qualified(
                                "sandbox credential preflight; OPEN accounts".into(),
                            ),
                        )?;
                        Some(accounts)
                    }
                    Err(error) => {
                        ledger.record(
                            "GetSandboxAccounts",
                            Evidence::Failed(adapter_failure(
                                "GetSandboxAccounts",
                                error.to_string(),
                            )),
                        )?;
                        None
                    }
                },
                Err(error) => {
                    ledger.record(
                        "GetSandboxAccounts",
                        Evidence::Failed(grpc_failure("GetSandboxAccounts", &error)),
                    )?;
                    None
                }
            };
            let selected = select_accounts(
                sandbox_accounts
                    .as_ref()
                    .map_or(&[][..], |catalogue| catalogue.accounts.as_slice()),
                AccountPurpose::GeneralRead,
            );
            if let Some(account) = selected.selected.first() {
                let account_id = account.account_id.clone();
                let _ = qualified(
                    &mut ledger,
                    "GetSandboxPortfolio",
                    sandbox
                        .get_sandbox_portfolio(v1::PortfolioRequest {
                            account_id: account_id.clone(),
                            currency: None,
                        })
                        .await,
                    PortfolioState::try_from,
                )?;
                let _ = qualified(
                    &mut ledger,
                    "GetSandboxPositions",
                    sandbox
                        .get_sandbox_positions(v1::PositionsRequest {
                            account_id: account_id.clone(),
                        })
                        .await,
                    PositionsState::try_from,
                )?;
                let _ = qualified(
                    &mut ledger,
                    "GetSandboxWithdrawLimits",
                    sandbox
                        .get_sandbox_withdraw_limits(v1::WithdrawLimitsRequest {
                            account_id: account_id.clone(),
                        })
                        .await,
                    WithdrawLimits::try_from,
                )?;
                let _ = qualified(
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
                    canonical_legacy_operations,
                )?;
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
            } else if sandbox_accounts.is_none() {
                for method in [
                    "GetSandboxPositions",
                    "GetSandboxOperations",
                    "GetSandboxOperationsByCursor",
                    "GetSandboxPortfolio",
                    "GetSandboxWithdrawLimits",
                ] {
                    ledger.record(
                        method,
                        Evidence::BlockedByPrerequisite("GetSandboxAccounts failed".into()),
                    )?;
                }
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
    let rows = ledger.finish()?;
    let summary = QualificationSummary::from_rows(&rows);
    for (method, evidence) in &rows {
        match evidence {
            Evidence::Qualified(detail) => println!("QUALIFIED {method}: {detail}"),
            Evidence::GatedUnavailable(detail) => {
                println!("GATED/UNAVAILABLE {method}: {detail}")
            }
            Evidence::BlockedByPrerequisite(detail) => {
                println!("BLOCKED_BY_PREREQUISITE {method}: {detail}")
            }
            Evidence::Failed(failure) => println!(
                "FAILED {method}: class={} grpc_status={:?} provider_code={} provider_message={} attempt={} tracking_id={} environment={} request_shape={} detail={}",
                failure.class.wire_name(),
                failure.grpc_status,
                failure.provider_code.as_deref().unwrap_or("-"),
                failure.provider_message.as_deref().unwrap_or("-"),
                failure
                    .attempt
                    .map_or_else(|| "-".into(), |attempt| attempt.to_string()),
                failure.tracking_id.as_deref().unwrap_or("-"),
                failure.environment.as_deref().unwrap_or("-"),
                failure.request_shape.as_deref().unwrap_or("-"),
                failure.detail
            ),
        }
    }
    println!(
        "COMPLETE SUMMARY qualified={} gated={} blocked={} failed={}",
        summary.qualified.len(),
        summary.gated.len(),
        summary.blocked.len(),
        summary.failed.len()
    );
    println!("QUALIFIED ROWS: {}", summary.qualified.join(", "));
    println!("GATED ROWS: {}", summary.gated.join(", "));
    println!("BLOCKED ROWS: {}", summary.blocked.join(", "));
    println!("FAILED ROWS: {}", summary.failed.join(", "));
    for (method, evidence) in &rows {
        if let Evidence::Failed(failure) = evidence {
            println!(
                "FAILED DETAIL {method}: grpc_status={:?} provider_code={} provider_message={} attempt={} tracking_id={} environment={} request_shape={}",
                failure.grpc_status,
                failure.provider_code.as_deref().unwrap_or("-"),
                failure.provider_message.as_deref().unwrap_or("-"),
                failure
                    .attempt
                    .map_or_else(|| "-".into(), |attempt| attempt.to_string()),
                failure.tracking_id.as_deref().unwrap_or("-"),
                failure.environment.as_deref().unwrap_or("-"),
                failure.request_shape.as_deref().unwrap_or("-")
            );
        }
    }
    summary.ensure_success()?;
    Ok(())
}

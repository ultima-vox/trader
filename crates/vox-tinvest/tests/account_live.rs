use std::error::Error;
use std::time::Duration;

use prost_types::Timestamp;
use tonic::Code;
use vox_tinvest::account::{
    AccountCatalogue, AccountValues, BankAccountCatalogue, MarginAttributes, PortfolioState,
    PositionsState, ProviderTimestamp, UserInfo, UserTariff, WithdrawLimits,
};
use vox_tinvest::generated::v1;
use vox_tinvest::operations::{OperationsFilter, OperationsPaginator, canonical_legacy_operations};
use vox_tinvest::operations_stream::{
    OperationsStreamConfig, OperationsStreamEvent, OperationsStreamSupervisor,
};
use vox_tinvest::reports::{BrokerReportState, ForeignIssuerReportState};
use vox_tinvest::{GrpcError, GrpcErrorKind, GrpcResponse, SecretToken, TInvestGrpcClient};

fn qualified<T>(
    method: &'static str,
    result: Result<GrpcResponse<T>, GrpcError>,
) -> Result<Option<T>, Box<dyn Error>> {
    match result {
        Ok(response) => {
            println!("QUALIFIED {method}");
            Ok(Some(response.body))
        }
        Err(error) if documented_gate(&error) => {
            println!("GATED/UNAVAILABLE {method}: {error}");
            Ok(None)
        }
        Err(error) => Err(Box::new(error)),
    }
}

fn documented_gate(error: &GrpcError) -> bool {
    matches!(
        &error.kind,
        GrpcErrorKind::Provider(provider)
            if matches!(provider.code, Code::PermissionDenied | Code::Unimplemented)
    )
}

fn recent_period() -> (Timestamp, Timestamp) {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    (
        Timestamp {
            seconds: now - 7 * 86_400,
            nanos: 0,
        },
        Timestamp {
            seconds: now,
            nanos: 0,
        },
    )
}

#[tokio::test]
#[ignore = "requires TINVEST_TOKEN; one complete read-only account qualification"]
async fn current_account_read_side_qualifies_over_grpc() -> Result<(), Box<dyn Error>> {
    let token = SecretToken::new(std::env::var("TINVEST_TOKEN")?)?;
    let client = TInvestGrpcClient::production(token.clone())?;
    let accounts = qualified(
        "GetAccounts",
        client.get_accounts(v1::GetAccountsRequest::default()).await,
    )?
    .ok_or("GetAccounts cannot be gated")?;
    let catalogue = AccountCatalogue::try_from(accounts)?;
    let account_ids = catalogue
        .accounts
        .iter()
        .map(|account| account.account_id.clone())
        .collect::<Vec<_>>();
    let account_id = account_ids
        .first()
        .ok_or("no production account available")?
        .clone();

    if let Some(body) = qualified(
        "GetMarginAttributes",
        client
            .get_margin_attributes(v1::GetMarginAttributesRequest {
                account_id: account_id.clone(),
            })
            .await,
    )? {
        let _ = MarginAttributes::try_from(body)?;
    }
    if let Some(body) = qualified(
        "GetUserTariff",
        client.get_user_tariff(v1::GetUserTariffRequest {}).await,
    )? {
        let _ = UserTariff::from(body);
    }
    if let Some(body) = qualified("GetInfo", client.get_info(v1::GetInfoRequest {}).await)? {
        let _ = UserInfo::from(body);
    }
    if let Some(body) = qualified(
        "GetBankAccounts",
        client
            .get_bank_accounts(v1::GetBankAccountsRequest {})
            .await,
    )? {
        let _ = BankAccountCatalogue::try_from(body)?;
    }
    if let Some(body) = qualified(
        "GetAccountValues",
        client
            .get_account_values(v1::GetAccountValuesRequest {
                accounts: account_ids.clone(),
                values: vec![
                    v1::AccountValue::MarginFee as i32,
                    v1::AccountValue::AmountWithoutExtraFee as i32,
                ],
            })
            .await,
    )? {
        let _ = AccountValues::try_from(body)?;
    }
    println!("NOT_CALLED CurrencyTransfer DEFERRED_MUTATION");
    println!("NOT_CALLED PayIn DEFERRED_MUTATION");

    let (from, to) = recent_period();
    if let Some(body) = qualified(
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
        "GetWithdrawLimits",
        client
            .get_withdraw_limits(v1::WithdrawLimitsRequest {
                account_id: account_id.clone(),
            })
            .await,
    )? {
        let _ = WithdrawLimits::try_from(body)?;
    }

    let history_request = OperationsFilter {
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
    };
    let history = OperationsPaginator::new(history_request)
        .map_err(Box::<dyn Error>::from)?
        .collect(&client)
        .await
        .map_err(Box::<dyn Error>::from)?;
    println!(
        "QUALIFIED GetOperationsByCursor pages={} items={}",
        history.pages,
        history.items.len()
    );

    let broker = v1::BrokerReportRequest {
        payload: Some(
            v1::broker_report_request::Payload::GenerateBrokerReportRequest(
                v1::GenerateBrokerReportRequest {
                    account_id: account_id.clone(),
                    from: Some(from),
                    to: Some(to),
                },
            ),
        ),
    };
    if let Some(body) = qualified("GetBrokerReport", client.get_broker_report(broker).await)? {
        let _ = BrokerReportState::try_from(body)?;
    }

    let from_time = time::OffsetDateTime::from_unix_timestamp(from.seconds)?;
    let year_start = time::Date::from_calendar_date(from_time.year(), time::Month::January, 1)?
        .with_hms(0, 0, 0)?
        .assume_utc()
        .unix_timestamp();
    let foreign = v1::GetDividendsForeignIssuerRequest {
        payload: Some(
            v1::get_dividends_foreign_issuer_request::Payload::GenerateDivForeignIssuerReport(
                v1::GenerateDividendsForeignIssuerReportRequest {
                    account_id: account_id.clone(),
                    from: Some(Timestamp {
                        seconds: from.seconds.max(year_start),
                        nanos: 0,
                    }),
                    to: Some(to),
                },
            ),
        ),
    };
    if let Some(body) = qualified(
        "GetDividendsForeignIssuer",
        client.get_dividends_foreign_issuer(foreign).await,
    )? {
        let _ = ForeignIssuerReportState::try_from(body)?;
    }

    let stream_config = OperationsStreamConfig {
        ping_delay_ms: 5_000,
        stale_timeout: Duration::from_secs(20),
        ..Default::default()
    };
    let supervisor = OperationsStreamSupervisor::new(client.clone(), stream_config)?;
    let mut stream = supervisor.start(account_ids)?;
    let mut subscribed = false;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(30), stream.recv())
            .await
            .map_err(Box::<dyn Error>::from)?
            .ok_or("operations stream event channel closed")?;
        match event {
            OperationsStreamEvent::Subscribed { .. } => subscribed = true,
            OperationsStreamEvent::Ping { .. } if subscribed => {
                println!("QUALIFIED OperationsStream subscription+ping");
                stream.stop();
                break;
            }
            OperationsStreamEvent::Fault(error) => return Err(Box::<dyn Error>::from(error)),
            _ => {}
        }
    }
    println!("GENERATED_COMPATIBILITY PortfolioStream");
    println!("GENERATED_COMPATIBILITY PositionsStream");

    let sandbox = TInvestGrpcClient::sandbox(token)?;
    let sandbox_accounts = qualified(
        "GetSandboxAccounts",
        sandbox
            .get_sandbox_accounts(v1::GetAccountsRequest::default())
            .await,
    )?
    .map(AccountCatalogue::try_from)
    .transpose()?;
    if let Some(sandbox_account_id) = sandbox_accounts
        .as_ref()
        .and_then(|catalogue| catalogue.accounts.first())
        .map(|account| account.account_id.clone())
    {
        if let Some(body) = qualified(
            "GetSandboxPortfolio",
            sandbox
                .get_sandbox_portfolio(v1::PortfolioRequest {
                    account_id: sandbox_account_id.clone(),
                    currency: None,
                })
                .await,
        )? {
            let _ = PortfolioState::try_from(body)?;
        }
        if let Some(body) = qualified(
            "GetSandboxPositions",
            sandbox
                .get_sandbox_positions(v1::PositionsRequest {
                    account_id: sandbox_account_id.clone(),
                })
                .await,
        )? {
            let _ = PositionsState::try_from(body)?;
        }
        if let Some(body) = qualified(
            "GetSandboxWithdrawLimits",
            sandbox
                .get_sandbox_withdraw_limits(v1::WithdrawLimitsRequest {
                    account_id: sandbox_account_id.clone(),
                })
                .await,
        )? {
            let _ = WithdrawLimits::try_from(body)?;
        }
        if let Some(body) = qualified(
            "GetSandboxOperations",
            sandbox
                .get_sandbox_operations(v1::OperationsRequest {
                    account_id: sandbox_account_id.clone(),
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
            account_id: sandbox_account_id,
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
        .await?;
        println!(
            "QUALIFIED GetSandboxOperationsByCursor pages={} items={}",
            history.pages,
            history.items.len()
        );
    } else {
        for method in [
            "GetSandboxPortfolio",
            "GetSandboxPositions",
            "GetSandboxWithdrawLimits",
            "GetSandboxOperations",
            "GetSandboxOperationsByCursor",
        ] {
            println!("GATED/UNAVAILABLE {method}: no sandbox account");
        }
    }
    println!("NOT_CALLED remaining SandboxService methods DEFERRED_MUTATION/ISSUE_10");
    Ok(())
}

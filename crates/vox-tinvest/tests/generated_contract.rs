use prost::Message;
use std::collections::BTreeSet;
use vox_domain::{FixedPoint, OrderSide, RegularOrderCommand, RegularOrderType, TimeInForce};
use vox_tinvest::TInvestGrpcClient;
use vox_tinvest::execution::{
    EXECUTION_STREAM_PAYLOADS, ExecutionPriceOperation, ORDERS_SERVICE_METHODS,
    ORDERS_STREAM_METHODS, STOP_ORDERS_SERVICE_METHODS, TInvestExecutionEnvironment,
    TInvestInstrumentKind, async_regular_order_request, execution_price_convention,
    regular_order_request,
};
use vox_tinvest::generated::v1;
use vox_tinvest::market_data::{MARKET_DATA_SERVICE_METHODS, MARKET_DATA_STREAM_METHODS};
use vox_tinvest::reference::INSTRUMENTS_SERVICE_METHODS;

const INSTRUMENTS_PROTO: &str = include_str!("../proto/tinkoff/instruments.proto");
const MARKET_DATA_PROTO: &str = include_str!("../proto/tinkoff/marketdata.proto");
const USERS_PROTO: &str = include_str!("../proto/tinkoff/users.proto");
const OPERATIONS_PROTO: &str = include_str!("../proto/tinkoff/operations.proto");
const SANDBOX_PROTO: &str = include_str!("../proto/tinkoff/sandbox.proto");
const ORDERS_PROTO: &str = include_str!("../proto/tinkoff/orders.proto");
const STOP_ORDERS_PROTO: &str = include_str!("../proto/tinkoff/stoporders.proto");

#[test]
fn generated_contract_inventory_matches_all_43_capability_rows() {
    let rpc_names = INSTRUMENTS_PROTO
        .lines()
        .filter_map(|line| line.trim().strip_prefix("rpc "))
        .filter_map(|line| line.split([' ', '(']).next())
        .collect::<Vec<_>>();
    assert_eq!(rpc_names.len(), 43);
    assert_eq!(INSTRUMENTS_SERVICE_METHODS.len(), 43);
    for rpc in rpc_names {
        assert!(
            INSTRUMENTS_SERVICE_METHODS.contains(&rpc),
            "generated provider RPC missing capability row: {rpc}"
        );
    }
}

#[test]
fn machine_matrix_matches_generated_rpc_inventory() {
    let matrix: serde_json::Value = serde_json::from_str(include_str!(
        "../../../qualification/tinvest_instruments_contracts.json"
    ))
    .expect("machine contract matrix must be valid JSON");
    assert_eq!(
        matrix["wire_strategy"]["kind"],
        "prost-tonic-generated-grpc"
    );
    let methods = matrix["methods"]
        .as_array()
        .expect("matrix methods must be an array");
    assert_eq!(methods.len(), 43);
    for method in INSTRUMENTS_SERVICE_METHODS {
        assert!(
            methods.iter().any(|row| row["method"] == *method),
            "generated RPC missing matrix row: {method}"
        );
    }
}

#[test]
fn proto_optional_scalar_and_message_presence_survive_round_trip() {
    let request = v1::NewsRequest {
        cursor: None,
        limit: None,
    };
    let decoded = v1::NewsRequest::decode(request.encode_to_vec().as_slice())
        .expect("generated request must decode");
    assert_eq!(decoded.cursor, None);
    assert_eq!(decoded.limit, None);

    let response = v1::GetFuturesMarginResponse::default();
    let decoded = v1::GetFuturesMarginResponse::decode(response.encode_to_vec().as_slice())
        .expect("generated response must decode");
    assert!(decoded.initial_margin_on_buy.is_none());
    assert!(decoded.initial_margin_on_sell.is_none());
    assert!(decoded.min_price_increment.is_none());
    assert!(decoded.min_price_increment_amount.is_none());
}

#[test]
fn unknown_enum_wire_number_is_preserved() {
    let source = v1::InstrumentShort {
        instrument_kind: 77_777,
        ..Default::default()
    };
    let decoded = v1::InstrumentShort::decode(source.encode_to_vec().as_slice())
        .expect("generated provider type must decode future enum number");
    assert_eq!(decoded.instrument_kind, 77_777);
}

#[test]
fn generated_market_data_inventory_matches_complete_official_contract() {
    let rpc_names = MARKET_DATA_PROTO
        .lines()
        .filter_map(|line| line.trim().strip_prefix("rpc "))
        .filter_map(|line| line.split([' ', '(']).next())
        .collect::<Vec<_>>();
    assert_eq!(rpc_names.len(), 11);
    for method in MARKET_DATA_SERVICE_METHODS
        .iter()
        .chain(MARKET_DATA_STREAM_METHODS.iter())
    {
        assert!(
            rpc_names.contains(method),
            "generated RPC missing: {method}"
        );
    }
}

#[test]
fn market_data_matrix_covers_methods_and_stream_oneofs() {
    let matrix: serde_json::Value = serde_json::from_str(include_str!(
        "../../../qualification/tinvest_market_data_contracts.json"
    ))
    .expect("machine market-data matrix must be valid JSON");
    assert_eq!(
        matrix["wire_strategy"]["kind"],
        "prost-tonic-generated-grpc"
    );
    assert_eq!(matrix["unary_methods"].as_array().map(Vec::len), Some(9));
    assert_eq!(matrix["stream_methods"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        matrix["stream_request_payloads"].as_array().map(Vec::len),
        Some(8)
    );
    assert_eq!(
        matrix["stream_response_payloads"].as_array().map(Vec::len),
        Some(12)
    );
}

#[test]
fn generated_market_data_presence_and_unknown_enums_survive_round_trip() {
    let candle = v1::HistoricCandle::default();
    let decoded = v1::HistoricCandle::decode(candle.encode_to_vec().as_slice())
        .expect("generated candle must decode");
    assert!(decoded.open.is_none());
    assert!(decoded.time.is_none());

    let trade = v1::Trade {
        direction: 77_777,
        ..Default::default()
    };
    let decoded = v1::Trade::decode(trade.encode_to_vec().as_slice())
        .expect("generated trade must decode future enum number");
    assert_eq!(decoded.direction, 77_777);
}

#[test]
fn account_read_side_matrix_matches_every_official_service_rpc() {
    let matrix: serde_json::Value = serde_json::from_str(include_str!(
        "../../../qualification/tinvest_account_contracts.json"
    ))
    .expect("account contract matrix must be valid JSON");
    assert_eq!(
        matrix["revision"],
        "762e720e27164213f41cac0b226c5698c2ae8199"
    );
    let rows = matrix["methods"]
        .as_array()
        .expect("account matrix methods must be an array");
    assert_eq!(rows.len(), 38);

    let official = rpc_inventory(USERS_PROTO)
        .into_iter()
        .chain(rpc_inventory(OPERATIONS_PROTO))
        .chain(rpc_inventory(SANDBOX_PROTO))
        .collect::<BTreeSet<_>>();
    let inventoried = rows
        .iter()
        .map(|row| {
            for required in [
                "service",
                "method",
                "request",
                "response",
                "class",
                "environment",
                "requirements",
                "identifiers",
                "pagination",
                "empty",
                "constraints",
                "adapter",
                "state",
                "routing",
                "canonical",
                "evidence",
                "limitations",
            ] {
                assert!(
                    row[required]
                        .as_str()
                        .is_some_and(|value| !value.is_empty()),
                    "inventory row missing {required}: {row}"
                );
            }
            let method = row["method"].as_str().expect("method");
            let expected_environment =
                vox_tinvest::account_qualification::method_environment(method)
                    .expect("inventory method environment classification")
                    .matrix_name();
            assert_eq!(
                row["environment"].as_str(),
                Some(expected_environment),
                "environment support drift for {method}"
            );
            (
                row["service"].as_str().expect("service").to_owned(),
                row["method"].as_str().expect("method").to_owned(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(inventoried.len(), rows.len(), "duplicate inventory row");
    assert_eq!(
        official, inventoried,
        "official account RPC inventory drift"
    );
}

#[test]
fn generated_regular_order_wire_requires_explicit_operation_price_type() {
    let command = |price_convention| RegularOrderCommand {
        account_id: "account".into(),
        instrument_id: "instrument".into(),
        client_request_id: "550e8400-e29b-41d4-a716-446655440030".into(),
        quantity_lots: 1,
        price: Some(FixedPoint::from_units_nano(100, 0).expect("price")),
        price_convention,
        side: OrderSide::Buy,
        order_type: RegularOrderType::Limit,
        time_in_force: Some(TimeInForce::Day),
        confirm_margin_trade: false,
    };
    let share = command(execution_price_convention(
        TInvestInstrumentKind::Share,
        ExecutionPriceOperation::RegularOrder,
        TInvestExecutionEnvironment::Production,
    ));
    assert_eq!(
        regular_order_request(&share).expect("share").price_type,
        v1::PriceType::Currency as i32
    );
    assert_eq!(
        async_regular_order_request(&share)
            .expect("share async")
            .price_type,
        Some(v1::PriceType::Currency as i32)
    );
    let future = command(execution_price_convention(
        TInvestInstrumentKind::Future,
        ExecutionPriceOperation::RegularOrder,
        TInvestExecutionEnvironment::Production,
    ));
    assert_eq!(
        regular_order_request(&future).expect("future").price_type,
        v1::PriceType::Point as i32
    );
    let sandbox_future = command(execution_price_convention(
        TInvestInstrumentKind::Future,
        ExecutionPriceOperation::RegularOrder,
        TInvestExecutionEnvironment::Sandbox,
    ));
    assert_eq!(
        regular_order_request(&sandbox_future)
            .expect("sandbox future")
            .price_type,
        v1::PriceType::Currency as i32
    );
}

fn rpc_inventory(proto: &str) -> Vec<(String, String)> {
    let mut service = None;
    let mut methods = Vec::new();
    for line in proto.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix("service ") {
            service = name.split_whitespace().next().map(str::to_owned);
        } else if line == "}" {
            service = None;
        } else if let (Some(service), Some(method)) = (
            service.as_ref(),
            line.strip_prefix("rpc ")
                .and_then(|rpc| rpc.split([' ', '(']).next()),
        ) {
            methods.push((service.clone(), method.to_owned()));
        }
    }
    methods
}

#[test]
fn execution_matrix_matches_every_pinned_rpc_and_stream_branch() {
    let matrix: serde_json::Value = serde_json::from_str(include_str!(
        "../../../qualification/tinvest_execution_contracts.json"
    ))
    .expect("execution contract matrix must be valid JSON");
    assert_eq!(
        matrix["revision"],
        "762e720e27164213f41cac0b226c5698c2ae8199"
    );
    assert!(
        matrix["common"]["price_conventions"]
            .as_str()
            .is_some_and(|value| value.contains("UNSPECIFIED is forbidden"))
    );
    let official = rpc_inventory(ORDERS_PROTO)
        .into_iter()
        .chain(rpc_inventory(STOP_ORDERS_PROTO))
        .chain(rpc_inventory(SANDBOX_PROTO))
        .collect::<BTreeSet<_>>();
    let rows = matrix["methods"].as_array().expect("method rows");
    let inventoried = rows
        .iter()
        .map(|row| {
            for field in [
                "service",
                "method",
                "request",
                "response",
                "class",
                "environment",
                "requirements",
                "identifiers",
                "idempotency",
                "variants",
                "constraints",
                "semantics",
                "adapter",
                "state",
                "routing",
                "canonical",
                "evidence",
                "qualification",
                "limitations",
            ] {
                assert!(
                    row[field].as_str().is_some_and(|value| !value.is_empty()),
                    "execution inventory row missing {field}: {row}"
                );
            }
            (
                row["service"].as_str().expect("service").to_owned(),
                row["method"].as_str().expect("method").to_owned(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        rows.len(),
        inventoried.len(),
        "duplicate execution inventory row"
    );
    assert_eq!(
        official, inventoried,
        "official execution RPC inventory drift"
    );

    let branches = matrix["stream_branches"]
        .as_array()
        .expect("stream branches");
    assert_eq!(branches.len(), EXECUTION_STREAM_PAYLOADS.len());
    for branch in EXECUTION_STREAM_PAYLOADS {
        let (stream, payload) = branch.split_once('.').expect("stream branch");
        assert!(
            branches
                .iter()
                .any(|row| { row["method"] == stream && row["branch"] == payload }),
            "official execution stream branch missing: {branch}"
        );
    }
    assert_eq!(ORDERS_SERVICE_METHODS.len(), 8);
    assert_eq!(ORDERS_STREAM_METHODS.len(), 2);
    assert_eq!(STOP_ORDERS_SERVICE_METHODS.len(), 3);
}

#[test]
fn every_execution_rpc_request_and_response_is_generated_and_round_trips() {
    macro_rules! round_trip {
        ($($type:ty),+ $(,)?) => {$({
            let value = <$type>::default();
            <$type>::decode(value.encode_to_vec().as_slice())
                .expect(concat!("generated round trip: ", stringify!($type)));
        })+};
    }
    round_trip!(
        v1::PostOrderRequest,
        v1::PostOrderResponse,
        v1::PostOrderAsyncRequest,
        v1::PostOrderAsyncResponse,
        v1::CancelOrderRequest,
        v1::CancelOrderResponse,
        v1::GetOrderStateRequest,
        v1::OrderState,
        v1::GetOrdersRequest,
        v1::GetOrdersResponse,
        v1::ReplaceOrderRequest,
        v1::GetMaxLotsRequest,
        v1::GetMaxLotsResponse,
        v1::GetOrderPriceRequest,
        v1::GetOrderPriceResponse,
        v1::TradesStreamRequest,
        v1::TradesStreamResponse,
        v1::OrderStateStreamRequest,
        v1::OrderStateStreamResponse,
        v1::PostStopOrderRequest,
        v1::PostStopOrderResponse,
        v1::GetStopOrdersRequest,
        v1::GetStopOrdersResponse,
        v1::CancelStopOrderRequest,
        v1::CancelStopOrderResponse,
    );
}

#[test]
fn generated_account_optionality_and_unknown_enums_survive_round_trip() {
    let account = v1::Account {
        r#type: 77_777,
        status: 88_888,
        access_level: 99_999,
        ..Default::default()
    };
    let decoded = v1::Account::decode(account.encode_to_vec().as_slice())
        .expect("generated account must decode");
    assert_eq!(decoded.r#type, 77_777);
    assert_eq!(decoded.status, 88_888);
    assert_eq!(decoded.access_level, 99_999);
    assert!(decoded.opened_date.is_none());
    assert!(decoded.closed_date.is_none());

    let operation = v1::OperationItem::default();
    let decoded = v1::OperationItem::decode(operation.encode_to_vec().as_slice())
        .expect("generated cursor operation must decode");
    assert!(decoded.date.is_none());
    assert!(decoded.payment.is_none());
    assert!(decoded.price.is_none());
}

#[allow(dead_code)]
fn adapter_exposes_every_unary_method(client: &TInvestGrpcClient) {
    drop(client.get_candles(v1::GetCandlesRequest::default()));
    drop(client.get_last_prices(v1::GetLastPricesRequest::default()));
    drop(client.get_order_book(v1::GetOrderBookRequest::default()));
    drop(client.get_trading_status(v1::GetTradingStatusRequest::default()));
    drop(client.get_trading_statuses(v1::GetTradingStatusesRequest::default()));
    drop(client.get_last_trades(v1::GetLastTradesRequest::default()));
    drop(client.get_close_prices(v1::GetClosePricesRequest::default()));
    drop(client.get_tech_analysis(v1::GetTechAnalysisRequest::default()));
    drop(client.get_market_values(v1::GetMarketValuesRequest::default()));
    drop(client.open_market_data_stream(
        1,
        vec![vox_tinvest::market_data::get_my_subscriptions_request()],
    ));
    drop(client.open_market_data_server_stream(v1::MarketDataServerSideStreamRequest::default()));
    drop(client.get_accounts(v1::GetAccountsRequest::default()));
    drop(client.get_margin_attributes(v1::GetMarginAttributesRequest::default()));
    drop(client.get_user_tariff(v1::GetUserTariffRequest::default()));
    drop(client.get_info(v1::GetInfoRequest::default()));
    drop(client.get_bank_accounts(v1::GetBankAccountsRequest::default()));
    drop(client.get_account_values(v1::GetAccountValuesRequest::default()));
    drop(client.get_operations(v1::OperationsRequest::default()));
    drop(client.get_portfolio(v1::PortfolioRequest::default()));
    drop(client.get_positions(v1::PositionsRequest::default()));
    drop(client.get_withdraw_limits(v1::WithdrawLimitsRequest::default()));
    drop(client.get_broker_report(v1::BrokerReportRequest::default()));
    drop(client.get_dividends_foreign_issuer(v1::GetDividendsForeignIssuerRequest::default()));
    drop(client.get_operations_by_cursor(v1::GetOperationsByCursorRequest::default()));
    drop(client.open_portfolio_stream(v1::PortfolioStreamRequest::default()));
    drop(client.open_positions_stream(v1::PositionsStreamRequest::default()));
    drop(client.open_operations_stream(v1::OperationsStreamRequest::default()));
    drop(client.get_sandbox_accounts(v1::GetAccountsRequest::default()));
    drop(client.get_sandbox_portfolio(v1::PortfolioRequest::default()));
    drop(client.get_sandbox_positions(v1::PositionsRequest::default()));
    drop(client.get_sandbox_withdraw_limits(v1::WithdrawLimitsRequest::default()));
    drop(client.get_sandbox_operations(v1::OperationsRequest::default()));
    drop(client.get_sandbox_operations_by_cursor(v1::GetOperationsByCursorRequest::default()));
}

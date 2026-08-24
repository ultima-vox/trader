use prost::Message;
use vox_tinvest::TInvestGrpcClient;
use vox_tinvest::generated::v1;
use vox_tinvest::market_data::{MARKET_DATA_SERVICE_METHODS, MARKET_DATA_STREAM_METHODS};
use vox_tinvest::reference::INSTRUMENTS_SERVICE_METHODS;

const INSTRUMENTS_PROTO: &str = include_str!("../proto/tinkoff/instruments.proto");
const MARKET_DATA_PROTO: &str = include_str!("../proto/tinkoff/marketdata.proto");

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
    drop(client.open_market_data_stream(1));
    drop(client.open_market_data_server_stream(v1::MarketDataServerSideStreamRequest::default()));
}

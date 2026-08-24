use prost::Message;
use vox_tinvest::generated::v1;
use vox_tinvest::reference::INSTRUMENTS_SERVICE_METHODS;

const INSTRUMENTS_PROTO: &str = include_str!("../proto/tinkoff/instruments.proto");

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

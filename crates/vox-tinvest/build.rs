use std::error::Error;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let proto_root = PathBuf::from("proto/tinkoff");
    let instrument_contract = proto_root.join("instruments.proto");
    let market_data_contract = proto_root.join("marketdata.proto");
    let operations_contract = proto_root.join("operations.proto");
    let orders_contract = proto_root.join("orders.proto");
    let sandbox_contract = proto_root.join("sandbox.proto");
    let stop_orders_contract = proto_root.join("stoporders.proto");
    let users_contract = proto_root.join("users.proto");
    let common_contract = proto_root.join("common.proto");

    println!("cargo:rerun-if-changed={}", instrument_contract.display());
    println!("cargo:rerun-if-changed={}", market_data_contract.display());
    println!("cargo:rerun-if-changed={}", operations_contract.display());
    println!("cargo:rerun-if-changed={}", orders_contract.display());
    println!("cargo:rerun-if-changed={}", sandbox_contract.display());
    println!("cargo:rerun-if-changed={}", stop_orders_contract.display());
    println!("cargo:rerun-if-changed={}", users_contract.display());
    println!("cargo:rerun-if-changed={}", common_contract.display());
    println!("cargo:rerun-if-changed=proto/tinkoff/google/api/field_behavior.proto");

    let mut prost = prost_build::Config::new();
    prost.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);

    tonic_build::configure()
        .build_client(true)
        .build_server(false)
        .compile_protos_with_config(
            prost,
            &[
                instrument_contract,
                market_data_contract,
                operations_contract,
                orders_contract,
                sandbox_contract,
                stop_orders_contract,
                users_contract,
                common_contract,
            ],
            &[proto_root],
        )?;
    Ok(())
}

use std::error::Error;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let proto_root = PathBuf::from("proto/tinkoff");
    let instrument_contract = proto_root.join("instruments.proto");
    let common_contract = proto_root.join("common.proto");

    println!("cargo:rerun-if-changed={}", instrument_contract.display());
    println!("cargo:rerun-if-changed={}", common_contract.display());
    println!("cargo:rerun-if-changed=proto/tinkoff/google/api/field_behavior.proto");

    let mut prost = prost_build::Config::new();
    prost.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);

    tonic_build::configure()
        .build_client(true)
        .build_server(false)
        .compile_protos_with_config(
            prost,
            &[instrument_contract, common_contract],
            &[proto_root],
        )?;
    Ok(())
}

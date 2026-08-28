//! Writes the OpenAPI document to stdout or to a file.
//!
//! `cargo run -p vox-api --bin openapi -- docs/api/openapi.json` regenerates the committed
//! artefact; CI runs the same command and fails if the working tree changes.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let json = vox_api::schema::openapi_json()?;
    match std::env::args().nth(1) {
        Some(path) => {
            std::fs::write(&path, format!("{json}\n"))?;
            eprintln!("wrote {path}");
        }
        None => println!("{json}"),
    }
    Ok(())
}

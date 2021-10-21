use std::io::Result;

fn main() -> Result<()> {
    tonic_build::configure()
        .build_server(true)
        .type_attribute(".", "#[derive(serde_derive::Serialize)]")
        .compile(&["api.proto"], &["src/api"])?;
    Ok(())
}

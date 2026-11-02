//! Generates disposable loopback TLS credentials for the CI-only updater journey.
fn main() -> anyhow::Result<()> {
    let directory = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("Expected a fixture directory"))?,
    );
    std::fs::create_dir_all(&directory)?;
    let identity =
        rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])?;
    std::fs::write(directory.join("certificate.der"), identity.cert.der())?;
    std::fs::write(
        directory.join("private.der"),
        identity.signing_key.serialize_der(),
    )?;
    Ok(())
}

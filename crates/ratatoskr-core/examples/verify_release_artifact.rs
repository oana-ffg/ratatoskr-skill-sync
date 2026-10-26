use anyhow::{Context, Result, ensure};
use base64::Engine;
use std::{env, fs};

fn main() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    ensure!(
        args.len() == 3,
        "Expected artifact, signature and release version"
    );
    let bytes = fs::read(&args[0]).context("Read release artifact")?;
    let signature = fs::read_to_string(&args[1]).context("Read release signature")?;
    ratatoskr_core::updates::verify(
        include_str!("../src/update-public-key.txt"),
        &bytes,
        &signature,
    )?;
    // The core verification authenticates the trusted comment as well as the payload.
    let decoded = base64::engine::general_purpose::STANDARD.decode(signature.trim())?;
    let signature = minisign_verify::Signature::decode(std::str::from_utf8(&decoded)?)?;
    verify_version(
        signature.trusted_comment(),
        args[2].to_str().context("Release version must be UTF-8")?,
    )?;
    println!("Verified release artifact, existing trust root and signed version");
    Ok(())
}

fn verify_version(comment: &str, expected: &str) -> Result<()> {
    let signed = comment
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"))
        .context("Signature does not bind a release version")?;
    ensure!(
        semver::Version::parse(signed.trim_start_matches('v'))?
            == semver::Version::parse(expected.trim_start_matches('v'))?,
        "Signature binds a different release version"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::verify_version;

    #[test]
    fn requires_an_exact_authenticated_version_field() {
        assert!(verify_version("timestamp:1\tfile:app\tversion:0.2.16", "v0.2.16").is_ok());
        assert!(verify_version("timestamp:1\tfile:app\tversion:0.2.15", "0.2.16").is_err());
        assert!(verify_version("timestamp:1\tfile:version:0.2.16", "0.2.16").is_err());
    }
}

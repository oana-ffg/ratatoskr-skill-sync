//! Credentials are resolved locally for each delivery; configuration contains references only.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Deserializer, Serialize};
use std::process::Command;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum SecretReference {
    Gopass { entry: String },
    Keychain { service: String, account: String },
    WindowsCredential { target: String },
    Environment { variable: String },
}

impl SecretReference {
    pub fn validate(&self) -> Result<()> {
        let name = |value: &str| -> Result<()> {
            ensure!(
                !value.trim().is_empty() && !value.chars().any(char::is_control),
                "Secret reference names must be nonempty and contain no control characters"
            );
            Ok(())
        };
        match self {
            Self::Gopass { entry } => {
                name(entry)?;
                ensure!(
                    !entry.starts_with('-'),
                    "Gopass entry must not start with a dash"
                );
            }
            Self::Keychain { service, account } => {
                name(service)?;
                name(account)?;
            }
            Self::WindowsCredential { target } => name(target)?,
            Self::Environment { variable } => {
                name(variable)?;
                ensure!(
                    variable.bytes().enumerate().all(|(i, b)| b == b'_'
                        || b.is_ascii_alphabetic()
                        || (i > 0 && b.is_ascii_digit())),
                    "Enter an environment variable name, not its value"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn resolve(&self) -> Result<String> {
        self.validate()?;
        let secret = match self {
            Self::Environment { variable } => environment_value(variable, std::env::var(variable))?,
            Self::Gopass { entry } => {
                let mut command = gopass_command()?;
                command.args(["show", "-o", entry]);
                command_secret(command, &format!("gopass entry {entry}"))?
            }
            Self::Keychain { service, account } => keychain(service, account)?,
            Self::WindowsCredential { target } => windows_credential(target)?,
        };
        ensure!(
            !secret.is_empty() && !secret.chars().any(char::is_control),
            "Resolved credential is empty or contains control characters"
        );
        Ok(secret)
    }
}

fn environment_value(
    variable: &str,
    value: std::result::Result<String, std::env::VarError>,
) -> Result<String> {
    // VarError::NotUnicode contains the value; never attach it as an error source.
    value.map_err(|_| {
        anyhow::anyhow!(
            "Resolve environment secret reference {variable}: variable unavailable or not UTF-8"
        )
    })
}

fn gopass_command() -> Result<Command> {
    // GUI/launchd PATH commonly omits Homebrew, including gpg used by gopass.
    #[cfg(target_os = "macos")]
    {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths: Vec<_> = std::env::split_paths(&path).collect();
        paths.extend(["/opt/homebrew/bin".into(), "/usr/local/bin".into()]);
        if let Some(program) = paths.iter().map(|p| p.join("gopass")).find(|p| p.is_file()) {
            let mut command = crate::process::command(program);
            command.env(
                "PATH",
                std::env::join_paths(paths).context("Prepare gopass search path")?,
            );
            return Ok(command);
        }
    }
    Ok(crate::process::command("gopass"))
}

fn command_secret(mut command: Command, reference: &str) -> Result<String> {
    command.stdin(std::process::Stdio::null());
    let output = command
        .output()
        .with_context(|| format!("Resolve {reference}: could not start secret store"))?;
    // Never include stdout/stderr: a failing provider may include credential bytes there.
    ensure!(
        output.status.success(),
        "Resolve {reference}: secret store exited with {}",
        output.status
    );
    let value = String::from_utf8(output.stdout)
        .map_err(|_| anyhow::anyhow!("Resolve {reference}: credential is not UTF-8"))?;
    Ok(value.trim_end_matches(['\r', '\n']).to_owned())
}

#[cfg(target_os = "macos")]
fn keychain(service: &str, account: &str) -> Result<String> {
    let mut command = crate::process::command("/usr/bin/security");
    command.args(["find-generic-password", "-s", service, "-a", account, "-w"]);
    command_secret(
        command,
        &format!("macOS Keychain item {service} / {account}"),
    )
}
#[cfg(not(target_os = "macos"))]
fn keychain(_: &str, _: &str) -> Result<String> {
    bail!("macOS Keychain references require macOS")
}

#[cfg(windows)]
fn windows_credential(target: &str) -> Result<String> {
    use windows_sys::Win32::Security::Credentials::{
        CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW,
    };
    let target: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
    // CredReadW owns the buffer until CredFree. Generic text passwords use UTF-16LE.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 {
        bail!(
            "Resolve Windows Credential Manager entry: {}",
            std::io::Error::last_os_error()
        );
    }
    let result = (|| {
        let credential = unsafe { &*credential };
        let size = credential.CredentialBlobSize as usize;
        ensure!(
            size > 0 && size.is_multiple_of(2),
            "Windows credential must be a UTF-16 text password"
        );
        let bytes = unsafe { std::slice::from_raw_parts(credential.CredentialBlob, size) };
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .copied()
            .map(u16::from_le_bytes)
            .collect();
        String::from_utf16(&units)
            .map_err(|_| anyhow::anyhow!("Windows credential is not valid UTF-16"))
    })();
    unsafe { CredFree(credential.cast()) };
    result
}
#[cfg(not(windows))]
fn windows_credential(_: &str) -> Result<String> {
    bail!("Windows Credential Manager references require Windows")
}

// Read v0.2.1 environment-name settings, but always serialize the explicit reference format.
pub(crate) fn optional_reference<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<SecretReference>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Compatible {
        Reference(SecretReference),
        Legacy(String),
    }
    Ok(
        Option::<Compatible>::deserialize(d)?.and_then(|value| match value {
            Compatible::Reference(reference) => Some(reference),
            Compatible::Legacy(variable) if variable.is_empty() => None,
            Compatible::Legacy(variable) => Some(SecretReference::Environment { variable }),
        }),
    )
}
pub(crate) fn required_reference<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<SecretReference, D::Error> {
    optional_reference(d)?.ok_or_else(|| serde::de::Error::custom("A secret reference is required"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn references_roundtrip_and_missing_values_never_enter_errors() {
        for text in [
            r#"{"kind":"gopass","entry":"alerts/app-token"}"#,
            r#"{"kind":"keychain","service":"alerts","account":"oana"}"#,
            r#"{"kind":"windows-credential","target":"ratatoskr-alerts"}"#,
            r#"{"kind":"environment","variable":"RATATOSKR_MISSING_SECRET_TEST"}"#,
        ] {
            let reference: SecretReference = serde_json::from_str(text).unwrap();
            reference.validate().unwrap();
            assert_eq!(serde_json::to_string(&reference).unwrap(), text);
        }
        let error = environment_value(
            "TEST_REFERENCE",
            Err(std::env::VarError::NotUnicode("private-value".into())),
        )
        .unwrap_err();
        assert!(!format!("{error:#}").contains("private-value"));
        let missing = SecretReference::Environment {
            variable: "RATATOSKR_MISSING_SECRET_TEST".into(),
        };
        assert!(
            missing
                .resolve()
                .unwrap_err()
                .to_string()
                .contains("variable unavailable")
        );
        assert!(
            SecretReference::Gopass {
                entry: "-bad-option".into()
            }
            .validate()
            .is_err()
        );
    }
    #[cfg(unix)]
    #[test]
    fn provider_output_is_consumed_on_success_and_redacted_on_failure() {
        let mut success = Command::new("sh");
        success.args(["-c", "printf 'test-credential\\n'"]);
        assert_eq!(
            command_secret(success, "test reference").unwrap(),
            "test-credential"
        );
        let mut failure = Command::new("sh");
        failure.args([
            "-c",
            "printf 'private-provider-output'; printf 'private-stderr' >&2; exit 1",
        ]);
        let error = command_secret(failure, "test reference")
            .unwrap_err()
            .to_string();
        assert!(error.contains("secret store exited"));
        assert!(!error.contains("private"));
    }
}

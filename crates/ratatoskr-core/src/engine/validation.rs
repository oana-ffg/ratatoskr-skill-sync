use super::{Output, RenderScope, render_scope_inner};
use crate::config::no_symlinks;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize)]
pub struct ValidationProblem {
    pub machine: String,
    pub harness: Option<String>,
    pub skill: Option<String>,
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct HarnessValidation {
    pub harness: String,
    pub skills_rendered: usize,
    pub skills_skipped: usize,
    pub files_rendered: usize,
    pub global_instructions_rendered: bool,
}

impl HarnessValidation {
    pub(super) fn new(harness: &str) -> Self {
        Self {
            harness: harness.into(),
            skills_rendered: 0,
            skills_skipped: 0,
            files_rendered: 0,
            global_instructions_rendered: false,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ProfileValidation {
    pub machine: String,
    pub harnesses: Vec<HarnessValidation>,
    pub problems: usize,
}

#[derive(Debug, Serialize)]
pub struct ValidationReport {
    pub source: PathBuf,
    pub profiles: Vec<ProfileValidation>,
    pub problems: Vec<ValidationProblem>,
}

#[derive(Default)]
pub(super) struct RenderDiagnostics {
    pub collect: bool,
    pub problems: Vec<ValidationProblem>,
    pub harnesses: Vec<HarnessValidation>,
}

impl RenderDiagnostics {
    /// Deployment fails immediately. Source validation records the same error and continues
    /// at the next independent file, skill, harness or profile.
    pub(super) fn capture<T>(
        &mut self,
        result: Result<T>,
        machine: &str,
        harness: Option<&str>,
        skill: Option<&str>,
        path: &Path,
    ) -> Result<Option<T>> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(error) if self.collect => {
                self.problems.push(ValidationProblem {
                    machine: machine.into(),
                    harness: harness.map(Into::into),
                    skill: skill.map(Into::into),
                    path: path.into(),
                    message: format!("{error:#}"),
                });
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
}

/// Render every source profile without configuration, Git, temporary files or output I/O.
/// Destination ownership and host-specific live-home checks belong to `plan`/`apply`.
pub fn validate_source(source: &Path) -> Result<ValidationReport> {
    let source = std::path::absolute(source)?;
    no_symlinks(&source)?;
    ensure!(
        source.is_dir(),
        "Source checkout does not exist: {}",
        source.display()
    );
    let source = source.canonicalize()?;
    let machines = source.join("machines");
    no_symlinks(&machines)?;
    let mut profiles = fs::read_dir(&machines)
        .with_context(|| format!("Read {}", machines.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    profiles.retain(|path| path.extension().is_some_and(|ext| ext == "toml"));
    profiles.sort();
    ensure!(!profiles.is_empty(), "No machines/*.toml profiles found");
    let mut report = ValidationReport {
        source: source.clone(),
        profiles: vec![],
        problems: vec![],
    };
    for path in profiles {
        let machine = path
            .file_stem()
            .context("Missing machine name")?
            .to_string_lossy()
            .into_owned();
        let mut diagnostics = RenderDiagnostics {
            collect: true,
            ..Default::default()
        };
        let result = render_scope_inner(
            &RenderScope {
                source: &source,
                machine: &machine,
                content: &source,
                project: None,
                output: Output::Validation,
            },
            &mut diagnostics,
        );
        diagnostics.capture(result, &machine, None, None, &path)?;
        report.profiles.push(ProfileValidation {
            machine,
            harnesses: diagnostics.harnesses,
            problems: diagnostics.problems.len(),
        });
        report.problems.extend(diagnostics.problems);
    }
    Ok(report)
}

// A foreign live profile's paths must be checked without asking the host filesystem
// to resolve its drive, home, permissions or symlinks.
pub(super) fn validate_root(root: &str, os: &str) -> Result<()> {
    let bytes = root.as_bytes();
    let absolute = if os == "windows" {
        (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
            || root.starts_with("\\\\")
            || root.starts_with("//")
    } else {
        root.starts_with('/')
    };
    ensure!(absolute, "Output root must be absolute for {os}: {root}");
    ensure!(
        !root.contains('\0') && !root.split(['/', '\\']).any(|part| part == ".."),
        "Unsafe output root: {root}"
    );
    Ok(())
}

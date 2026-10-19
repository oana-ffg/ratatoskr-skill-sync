use super::{Output, RenderScope, render_scope_inner};
use crate::config::no_symlinks;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ValidationContext {
    pub machine: String,
    pub harness: Option<String>,
    pub skill: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ValidationProblem {
    /// Source-relative repository path with forward slashes on every OS.
    pub path: String,
    pub message: String,
    pub contexts: Vec<ValidationContext>,
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
    /// Distinct problem/context pairs affecting this profile.
    pub problem_occurrences: usize,
}

#[derive(Debug, Serialize)]
pub struct ValidationReport {
    pub source: PathBuf,
    pub profiles: Vec<ProfileValidation>,
    /// Total problem/context pairs across all profiles, equal to the sum of profile counts.
    pub problem_occurrences: usize,
    /// Unique (path, message) problems with every affected machine/harness/skill context.
    pub problems: Vec<ValidationProblem>,
}

#[derive(Default)]
pub(super) struct RenderDiagnostics {
    pub collect: bool,
    problems: BTreeMap<(PathBuf, String), BTreeSet<ValidationContext>>,
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
                self.problems
                    .entry((path.into(), format!("{error:#}")))
                    .or_default()
                    .insert(ValidationContext {
                        machine: machine.into(),
                        harness: harness.map(Into::into),
                        skill: skill.map(Into::into),
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
    let mut summaries = vec![];
    let mut problems: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
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
        summaries.push(ProfileValidation {
            machine,
            harnesses: diagnostics.harnesses,
            problem_occurrences: diagnostics.problems.values().map(BTreeSet::len).sum(),
        });
        for ((path, message), contexts) in diagnostics.problems {
            let path = path
                .strip_prefix(&source)?
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            problems
                .entry((path, message))
                .or_default()
                .extend(contexts);
        }
    }
    Ok(ValidationReport {
        source,
        problem_occurrences: summaries.iter().map(|p| p.problem_occurrences).sum(),
        profiles: summaries,
        problems: problems
            .into_iter()
            .map(|((path, message), contexts)| ValidationProblem {
                path,
                message,
                contexts: contexts.into_iter().collect(),
            })
            .collect(),
    })
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

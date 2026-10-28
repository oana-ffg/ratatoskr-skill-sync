use super::{Output, RenderScope, render_scope_inner};
use crate::config::no_symlinks;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

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
    /// Non-fatal findings, such as selector values that no profile or harness can match.
    pub warnings: Vec<ValidationWarning>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ValidationWarning {
    /// Source-relative repository path with forward slashes on every OS.
    pub path: String,
    pub message: String,
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
    let warnings = selector_vocabulary_problems(&source, &summaries);
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
        warnings,
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

const KNOWN_OS: [&str; 3] = ["linux", "macos", "windows"];

/// Selector values that can never match, such as `harness=cursro` or `os=mac`, silently drop
/// content on every profile. Report them even though each individual render succeeds.
struct SelectorVocabulary {
    harnesses: BTreeSet<String>,
    machines: BTreeSet<String>,
    seen: BTreeSet<(String, String)>,
    warnings: Vec<ValidationWarning>,
}

impl SelectorVocabulary {
    fn unknown(&self, key: &str, value: &str) -> Option<String> {
        let (known, label): (Vec<&str>, &str) = match key {
            "harness" | "harnesses" => (
                self.harnesses.iter().map(String::as_str).collect(),
                "harness",
            ),
            "machine" | "machines" => (
                self.machines.iter().map(String::as_str).collect(),
                "machine",
            ),
            "os" => (KNOWN_OS.to_vec(), "os"),
            _ => return None,
        };
        if known.contains(&value) {
            return None;
        }
        let list = if known.is_empty() {
            "none".to_string()
        } else {
            known.join(", ")
        };
        Some(format!(
            "Unknown {label} `{value}` never matches; known: {list}"
        ))
    }

    fn push(&mut self, path: impl Into<String>, message: String) {
        let path = path.into();
        if self.seen.insert((path.clone(), message.clone())) {
            self.warnings.push(ValidationWarning { path, message });
        }
    }

    fn warn_markdown(&mut self, path: &Path, relative: &str) {
        let Ok(text) = fs::read_to_string(path) else {
            return;
        };
        let Ok((blocks, _)) = crate::template::parse_blocks(&text) else {
            return;
        };
        for block in blocks {
            for (key, values) in &block.selectors {
                for value in values {
                    if let Some(message) = self.unknown(key, value) {
                        self.push(relative, message);
                    }
                }
            }
        }
    }

    fn warn_skill_markdown(&mut self, skill_root: &Path, name: &str) {
        let mut files = Vec::new();
        for entry in WalkDir::new(skill_root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            if super::runtime(entry.path()) || entry.path().extension() != Some("md".as_ref()) {
                continue;
            }
            if let Ok(relative) = entry.path().strip_prefix(skill_root) {
                files.push(relative.to_path_buf());
            }
        }
        files.sort();
        for relative in files {
            let Some(portable) = relative.to_str() else {
                continue;
            };
            self.warn_markdown(
                &skill_root.join(&relative),
                &format!("skills/{name}/{}", portable.replace('\\', "/")),
            );
        }
    }

    fn warn_target(&mut self, skill_root: &Path, name: &str) {
        let Some(target) = fs::read_to_string(skill_root.join("skill.toml"))
            .ok()
            .and_then(|text| text.parse::<toml::Table>().ok())
            .and_then(|table| table.get("target").and_then(toml::Value::as_table).cloned())
        else {
            return;
        };
        for (key, values) in &target {
            let Some(values) = values.as_array() else {
                continue;
            };
            for value in values.iter().filter_map(toml::Value::as_str) {
                if let Some(message) = self.unknown(key, value) {
                    self.push(format!("skills/{name}/skill.toml"), message);
                }
            }
        }
    }

    fn warn_variants(&mut self, skill_root: &Path, name: &str) {
        let Ok(entries) = fs::read_dir(skill_root.join("variants")) else {
            return;
        };
        let mut variants: Vec<_> = entries.filter_map(Result::ok).collect();
        variants.sort_by_key(|entry| entry.file_name());
        for variant in variants {
            let Ok(kind) = variant.file_type() else {
                continue;
            };
            if kind.is_symlink() || !kind.is_dir() {
                continue;
            }
            let label = variant.file_name().to_string_lossy().into_owned();
            if let Some((key, value)) = label.split_once('=')
                && let Some(message) = self.unknown(key, value)
            {
                self.push(format!("skills/{name}/variants/{label}"), message);
            }
        }
    }
}

fn selector_vocabulary_problems(
    source: &Path,
    profiles: &[ProfileValidation],
) -> Vec<ValidationWarning> {
    let harnesses = fs::read_to_string(source.join("harnesses.toml"))
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .map(|table| table.keys().cloned().collect())
        .unwrap_or_default();
    let mut vocabulary = SelectorVocabulary {
        harnesses,
        machines: profiles
            .iter()
            .map(|profile| profile.machine.clone())
            .collect(),
        seen: BTreeSet::new(),
        warnings: Vec::new(),
    };
    vocabulary.warn_markdown(
        &source.join(crate::content::GLOBAL_INSTRUCTIONS),
        crate::content::GLOBAL_INSTRUCTIONS,
    );
    let Ok(entries) = fs::read_dir(source.join("skills")) else {
        return vocabulary.warnings;
    };
    let mut skills: Vec<_> = entries.filter_map(Result::ok).collect();
    skills.sort_by_key(|entry| entry.file_name());
    for skill in skills {
        let Ok(kind) = skill.file_type() else {
            continue;
        };
        if kind.is_symlink() || !kind.is_dir() || super::runtime(&skill.path()) {
            continue;
        }
        let name = skill.file_name().to_string_lossy().into_owned();
        let root = skill.path();
        vocabulary.warn_skill_markdown(&root, &name);
        vocabulary.warn_target(&root, &name);
        vocabulary.warn_variants(&root, &name);
    }
    vocabulary.warnings
}

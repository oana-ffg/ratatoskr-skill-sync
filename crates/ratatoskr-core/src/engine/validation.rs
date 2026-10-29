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

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
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
    let scan = selector_vocabulary_problems(&source, &summaries);
    let profile_contexts: Vec<ValidationContext> = summaries
        .iter()
        .map(|profile| ValidationContext {
            machine: profile.machine.clone(),
            harness: None,
            skill: None,
        })
        .collect();
    for (path, message) in scan.inspections {
        problems
            .entry((path, message))
            .or_default()
            .extend(profile_contexts.iter().cloned());
        for profile in &mut summaries {
            profile.problem_occurrences += 1;
        }
    }
    let warnings = scan.warnings;
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

/// Selector values that can never match, such as `harness=cursro` or `os=mac`, silently drop
/// content on every profile. Report them even though each individual render succeeds.
/// `harnesses` is `None` when `harnesses.toml` itself could not be read; harness warnings are
/// skipped in that case so a missing vocabulary is not reported as a typo.
struct SelectorVocabulary {
    harnesses: Option<BTreeSet<String>>,
    machines: BTreeSet<String>,
    seen: BTreeSet<(String, String)>,
    warnings: Vec<ValidationWarning>,
    inspections: Vec<(String, String)>,
}

struct SelectorScan {
    warnings: Vec<ValidationWarning>,
    inspections: Vec<(String, String)>,
}

impl SelectorVocabulary {
    fn unknown(&self, key: &str, value: &str) -> Option<String> {
        let (known, label): (Option<Vec<&str>>, &str) = match key {
            "harness" | "harnesses" => (
                self.harnesses
                    .as_ref()
                    .map(|names| names.iter().map(String::as_str).collect()),
                "harness",
            ),
            "machine" | "machines" => (
                Some(self.machines.iter().map(String::as_str).collect()),
                "machine",
            ),
            "os" => (Some(crate::SUPPORTED_OS.to_vec()), "os"),
            _ => return None,
        };
        let known = known?;
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

    fn inspect_fail(&mut self, path: &str, detail: &str) {
        let message = format!("Could not inspect {path}: {detail}");
        if self.seen.insert((path.to_string(), message.clone())) {
            self.inspections.push((path.to_string(), message));
        }
    }

    fn parse_fail(&mut self, path: &str, detail: &str) {
        let message = format!("Could not parse {path}: {detail}");
        if self.seen.insert((path.to_string(), message.clone())) {
            self.inspections.push((path.to_string(), message));
        }
    }

    /// `None` for a missing optional path. Any other I/O error is recorded.
    fn read_optional(&mut self, relative: &str, path: &Path) -> Option<String> {
        match fs::read_to_string(path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                self.inspect_fail(relative, &error.to_string());
                None
            }
        }
    }

    fn read_required(&mut self, relative: &str, path: &Path) -> Option<String> {
        match fs::read_to_string(path) {
            Ok(text) => Some(text),
            Err(error) => {
                self.inspect_fail(relative, &error.to_string());
                None
            }
        }
    }

    /// `None` when the directory is absent. Present directories yield every entry, and a failed
    /// entry is recorded instead of being dropped.
    fn read_dir_optional(&mut self, relative: &str, path: &Path) -> Option<Vec<fs::DirEntry>> {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                self.inspect_fail(relative, &error.to_string());
                return None;
            }
        };
        let mut collected = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => collected.push(entry),
                Err(error) => self.inspect_fail(relative, &error.to_string()),
            }
        }
        collected.sort_by_key(|entry| entry.file_name());
        Some(collected)
    }

    fn push(&mut self, path: impl Into<String>, message: String) {
        let path = path.into();
        if self.seen.insert((path.clone(), message.clone())) {
            self.warnings.push(ValidationWarning { path, message });
        }
    }

    fn warn_markdown_text(&mut self, relative: &str, text: &str) {
        let (blocks, diagnostics) = match crate::template::parse_blocks(text) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.parse_fail(relative, &format!("{error:#}"));
                return;
            }
        };
        for diagnostic in diagnostics {
            if diagnostic.severity == "error" {
                self.parse_fail(relative, &diagnostic.message);
            }
        }
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

    fn warn_optional_markdown(&mut self, relative: &str, path: &Path) {
        if let Some(text) = self.read_optional(relative, path) {
            self.warn_markdown_text(relative, &text);
        }
    }

    fn relative_under(&mut self, root: &Path, path: &Path, prefix: &str) -> Option<String> {
        let Ok(relative) = path.strip_prefix(root) else {
            self.inspect_fail(prefix, "path escaped the skill directory");
            return None;
        };
        let Some(portable) = relative.to_str() else {
            self.inspect_fail(prefix, "non-UTF8 file name");
            return None;
        };
        let portable = portable.replace('\\', "/");
        Some(if portable.is_empty() {
            prefix.to_string()
        } else {
            format!("{prefix}/{portable}")
        })
    }

    fn warn_skill_markdown(&mut self, skill_root: &Path, name: &str) {
        let prefix = format!("skills/{name}");
        let mut files = Vec::new();
        for entry in WalkDir::new(skill_root).follow_links(false) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    let relative = error
                        .path()
                        .and_then(|path| self.relative_under(skill_root, path, &prefix))
                        .unwrap_or_else(|| prefix.clone());
                    let detail = error
                        .io_error()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| error.to_string());
                    self.inspect_fail(&relative, &detail);
                    continue;
                }
            };
            let Some(relative) = self.relative_under(skill_root, entry.path(), &prefix) else {
                continue;
            };
            if entry.path() == skill_root {
                continue;
            }
            if entry.file_type().is_symlink() {
                self.inspect_fail(&relative, "symlink");
                continue;
            }
            if !entry.file_type().is_file()
                || super::runtime(entry.path())
                || entry.path().extension() != Some("md".as_ref())
            {
                continue;
            }
            files.push((relative, entry.path().to_path_buf()));
        }
        files.sort();
        for (relative, path) in files {
            if let Some(text) = self.read_required(&relative, &path) {
                self.warn_markdown_text(&relative, &text);
            }
        }
    }

    fn warn_target(&mut self, skill_root: &Path, name: &str) {
        let relative = format!("skills/{name}/skill.toml");
        let Some(text) = self.read_required(&relative, &skill_root.join("skill.toml")) else {
            return;
        };
        let table = match text.parse::<toml::Table>() {
            Ok(table) => table,
            Err(error) => {
                self.parse_fail(&relative, &error.to_string());
                return;
            }
        };
        let Some(target) = table.get("target") else {
            return;
        };
        let Some(target) = target.as_table() else {
            self.parse_fail(&relative, "target is not a table");
            return;
        };
        for (key, values) in target {
            if !matches!(
                key.as_str(),
                "harnesses" | "harness" | "machines" | "machine" | "os"
            ) {
                continue;
            }
            let Some(values) = values.as_array() else {
                self.parse_fail(&relative, &format!("target `{key}` is not an array"));
                continue;
            };
            for value in values {
                let Some(value) = value.as_str() else {
                    self.parse_fail(&relative, &format!("target `{key}` contains a non-string"));
                    continue;
                };
                if let Some(message) = self.unknown(key, value) {
                    self.push(relative.clone(), message);
                }
            }
        }
    }

    fn warn_variants(&mut self, skill_root: &Path, name: &str) {
        let relative = format!("skills/{name}/variants");
        let Some(variants) = self.read_dir_optional(&relative, &skill_root.join("variants")) else {
            return;
        };
        for variant in variants {
            match super::variant_runtime_entry(&variant) {
                Ok(true) => continue,
                Ok(false) => (),
                Err(error) => {
                    self.inspect_fail(&relative, &error.to_string());
                    continue;
                }
            }
            let Some(label) = variant.file_name().into_string().ok() else {
                self.inspect_fail(&relative, "non-UTF8 file name");
                continue;
            };
            let variant_path = format!("{relative}/{label}");
            let kind = match variant.file_type() {
                Ok(kind) => kind,
                Err(error) => {
                    self.inspect_fail(&variant_path, &error.to_string());
                    continue;
                }
            };
            if kind.is_symlink() {
                self.inspect_fail(&variant_path, "symlink");
                continue;
            }
            if !kind.is_dir() {
                self.inspect_fail(&variant_path, "variant is not a directory");
                continue;
            }
            if let Some((key, value)) = label.split_once('=')
                && let Some(message) = self.unknown(key, value)
            {
                self.push(variant_path, message);
            }
        }
    }
}

fn selector_vocabulary_problems(source: &Path, profiles: &[ProfileValidation]) -> SelectorScan {
    let mut vocabulary = SelectorVocabulary {
        harnesses: None,
        machines: profiles
            .iter()
            .map(|profile| profile.machine.clone())
            .collect(),
        seen: BTreeSet::new(),
        warnings: Vec::new(),
        inspections: Vec::new(),
    };
    let harnesses_path = source.join("harnesses.toml");
    if let Some(text) = vocabulary.read_required("harnesses.toml", &harnesses_path) {
        match text.parse::<toml::Table>() {
            Ok(table) => vocabulary.harnesses = Some(table.keys().cloned().collect()),
            Err(error) => vocabulary.parse_fail("harnesses.toml", &error.to_string()),
        }
    }
    vocabulary.warn_optional_markdown(
        crate::content::GLOBAL_INSTRUCTIONS,
        &source.join(crate::content::GLOBAL_INSTRUCTIONS),
    );
    if let Some(skills) = vocabulary.read_dir_optional("skills", &source.join("skills")) {
        for skill in skills {
            let Some(name) = skill.file_name().into_string().ok() else {
                vocabulary.inspect_fail("skills", "non-UTF8 file name");
                continue;
            };
            let skill_path = format!("skills/{name}");
            let kind = match skill.file_type() {
                Ok(kind) => kind,
                Err(error) => {
                    vocabulary.inspect_fail(&skill_path, &error.to_string());
                    continue;
                }
            };
            if kind.is_symlink() {
                vocabulary.inspect_fail(&skill_path, "symlink");
                continue;
            }
            if !kind.is_dir() || super::runtime(&skill.path()) {
                continue;
            }
            let root = skill.path();
            vocabulary.warn_skill_markdown(&root, &name);
            vocabulary.warn_target(&root, &name);
            vocabulary.warn_variants(&root, &name);
        }
    }
    SelectorScan {
        warnings: vocabulary.warnings,
        inspections: vocabulary.inspections,
    }
}

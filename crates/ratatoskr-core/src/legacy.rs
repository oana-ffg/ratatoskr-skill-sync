//! Preview and explicitly archive duplicate legacy Codex skills without deleting bytes.
use crate::{
    Settings,
    config::{identifier, no_symlinks},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, PartialEq)]
pub struct LegacyFile {
    pub path: PathBuf,
    pub sha256: String,
    pub executable: bool,
}
#[derive(Serialize, Deserialize, PartialEq)]
pub struct LegacySkill {
    pub name: String,
    pub path: PathBuf,
    pub files: Vec<LegacyFile>,
}

pub fn legacy_duplicates(settings: &Settings) -> Result<Vec<LegacySkill>> {
    let base = directories::BaseDirs::new().context("Home directory unavailable")?;
    duplicates_in(settings, base.home_dir())
}
fn duplicates_in(settings: &Settings, actual_home: &Path) -> Result<Vec<LegacySkill>> {
    let contexts = crate::engine::template_contexts(settings)?;
    let Some((_, _, context)) = contexts
        .iter()
        .find(|(m, h, _)| m == &settings.machine && h == "codex")
    else {
        return Ok(vec![]);
    };
    let Some(home) = context.get("home").and_then(|v| v.as_str()) else {
        return Ok(vec![]);
    };
    // Only inspect the local user's legacy installation, even when previewing other profiles.
    if PathBuf::from(home).canonicalize()? != actual_home.canonicalize()? {
        return Ok(vec![]);
    }
    let definitions = crate::structured_source(settings, "harnesses.toml")?;
    let codex = definitions
        .values
        .get("codex")
        .context("Codex target missing")?;
    let os = context
        .get("os")
        .and_then(|v| v.as_str())
        .context("Machine OS missing")?;
    let root = codex
        .get("roots")
        .and_then(|v| v.get(os))
        .and_then(|v| v.as_str())
        .context("Codex root missing")?;
    let destination = PathBuf::from(crate::template::render(root, context, false)?).join(
        codex
            .get("skills_dir")
            .and_then(|v| v.as_str())
            .context("Codex skills folder missing")?,
    );
    let legacy = actual_home.join(".codex/skills");
    if destination == legacy {
        return Ok(vec![]);
    }
    no_symlinks(&legacy)?;
    let mut out = vec![];
    for skill in crate::list_skills(settings)?
        .into_iter()
        .filter(|s| s.project.is_none() && s.enabled)
    {
        let document = crate::read_skill(settings, None, &skill.name)?;
        let meta: toml::Table = toml::from_str(&document.metadata)?;
        if let Some(target) = meta.get("target")
            && !crate::template::selectors_match(
                target.as_table().context("Target must be a table")?,
                context,
            )?
        {
            continue;
        }
        let path = legacy.join(&skill.name);
        no_symlinks(&path)?;
        if !path.try_exists()? {
            continue;
        }
        ensure!(
            path.is_dir(),
            "Legacy skill is not a directory: {}",
            path.display()
        );
        // file_paths intentionally ignores runtime artifacts; legacy backups must enumerate ALL bytes.
        let mut files = vec![];
        for entry in walkdir::WalkDir::new(&path).follow_links(false) {
            let entry = entry?;
            ensure!(
                !entry.file_type().is_symlink(),
                "Legacy skill contains a symlink: {}",
                entry.path().display()
            );
            if entry.file_type().is_dir() {
                continue;
            }
            ensure!(entry.file_type().is_file(), "Unsupported legacy file");
            #[cfg(unix)]
            let executable = {
                use std::os::unix::fs::PermissionsExt;
                entry.metadata()?.permissions().mode() & 0o111 != 0
            };
            #[cfg(not(unix))]
            let executable = false;
            files.push(LegacyFile {
                path: entry.path().strip_prefix(&path)?.into(),
                sha256: format!("{:x}", Sha256::digest(fs::read(entry.path())?)),
                executable,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        out.push(LegacySkill {
            name: skill.name,
            path,
            files,
        });
    }
    Ok(out)
}
#[derive(Serialize, Deserialize, PartialEq)]
pub struct LegacyResolution {
    pub skill: LegacySkill,
    pub backup: PathBuf,
    pub ownership: Option<String>,
}
pub fn preview_legacy_resolution(settings: &Settings, name: &str) -> Result<LegacyResolution> {
    identifier(name)?;
    let skill = legacy_duplicates(settings)?
        .into_iter()
        .find(|s| s.name == name)
        .context("Legacy duplicate no longer exists")?;
    // Same-volume rename is lossless, including permissions, empty directories and runtime artifacts.
    let backup = skill
        .path
        .parent()
        .context("Legacy parent missing")?
        .parent()
        .context("Codex home missing")?
        .join("ratatoskr-backups")
        .join(format!(
            "{}-{name}",
            chrono::Utc::now().format("%Y%m%dT%H%M%S%f")
        ));
    no_symlinks(&backup)?;
    let ownership = crate::engine::legacy_ownership(settings, &skill.path)?;
    Ok(LegacyResolution {
        skill,
        backup,
        ownership,
    })
}
pub fn resolve_legacy(settings: &Settings, expected: &str) -> Result<PathBuf> {
    let base = directories::BaseDirs::new().context("Home directory unavailable")?;
    resolve_in(settings, expected, base.home_dir())
}
fn resolve_in(settings: &Settings, expected: &str, actual_home: &Path) -> Result<PathBuf> {
    let _lock = crate::config::operation_lock(settings, "apply.lock")?;
    let expected: LegacyResolution = serde_json::from_str(expected)?;
    let current = duplicates_in(settings, actual_home)?
        .into_iter()
        .find(|s| s.name == expected.skill.name)
        .context("Legacy duplicate no longer exists")?;
    ensure!(
        current == expected.skill,
        "Legacy skill changed since preview; review it again"
    );
    ensure!(
        crate::engine::legacy_ownership(settings, &current.path)? == expected.ownership,
        "Legacy ownership changed since preview; review it again"
    );
    let parent = current
        .path
        .parent()
        .context("Missing parent")?
        .parent()
        .context("Missing Codex home")?
        .join("ratatoskr-backups");
    ensure!(
        expected.backup.parent() == Some(parent.as_path()),
        "Backup is outside the approved archive folder"
    );
    no_symlinks(&expected.backup)?;
    ensure!(!expected.backup.try_exists()?, "Backup already exists");
    fs::create_dir_all(&parent)?;
    // Persist the full reviewed fingerprint beside the archived directory for manual recovery.
    let receipt = expected.backup.with_extension("json");
    ensure!(!receipt.try_exists()?, "Backup receipt already exists");
    crate::config::atomic_write(
        &receipt,
        serde_json::to_string_pretty(&expected)?.as_bytes(),
    )?;
    let recheck = duplicates_in(settings, actual_home)?
        .into_iter()
        .find(|s| s.name == current.name);
    ensure!(
        recheck.as_ref() == Some(&current),
        "Legacy skill changed during backup; review again"
    );
    fs::rename(&current.path, &expected.backup)
        .context("Archive legacy skill (nothing deleted)")?;
    if let Some(ownership) = &expected.ownership
        && let Err(error) =
            crate::engine::detach_legacy_ownership(settings, &current.path, ownership)
    {
        fs::rename(&expected.backup, &current.path)
            .context("Ownership update failed and archive rollback failed")?;
        return Err(error);
    }
    Ok(expected.backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codex_cutover_previews_all_bytes_and_refuses_stale_or_redirected_backups() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let home = root.join("home");
        fs::create_dir_all(&home)?;
        let source = root.join("content");
        fs::create_dir_all(source.join("machines"))?;
        fs::create_dir_all(source.join("skills/review"))?;
        fs::write(
            source.join("harnesses.toml"),
            "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\n[codex.roots]\nmacos='{{af:home}}/.agents'\nwindows='{{af:home}}/.agents'\nlinux='{{af:home}}/.agents'\n",
        )?;
        let machine = toml::Table::from_iter([
            ("home".into(), home.to_string_lossy().into_owned().into()),
            ("os".into(), "macos".into()),
            ("harnesses".into(), toml::Value::try_from(vec!["codex"])?),
        ]);
        fs::write(
            source.join("machines/test.toml"),
            toml::to_string(&machine)?,
        )?;
        fs::write(source.join("skills/review/skill.toml"), "")?;
        fs::write(
            source.join("skills/review/SKILL.md"),
            "---\nname: review\ndescription: Review code\n---\nRead carefully.\n",
        )?;
        let legacy = home.join(".codex/skills/review");
        fs::create_dir_all(legacy.join("__pycache__"))?;
        fs::write(legacy.join("SKILL.md"), "user edit")?;
        fs::write(legacy.join("__pycache__/keep.pyc"), [0, 1, 255])?;
        let settings = Settings {
            source,
            machine: "test".into(),
            state_dir: root.join("state"),
            ..Default::default()
        };
        let manifest = settings.state_dir.join("test/manifest.json");
        fs::create_dir_all(manifest.parent().unwrap())?;
        let owned = serde_json::json!({"source":settings.source,"machine":"test","roots":[legacy],"files":{(legacy.join("SKILL.md").to_string_lossy().to_string()):{"sha256":format!("{:x}",Sha256::digest(b"user edit")),"executable":false}},"project_roots":{}});
        fs::write(&manifest, serde_json::to_vec(&owned)?)?;
        let mut preview = LegacyResolution {
            skill: duplicates_in(&settings, &home)?.remove(0),
            backup: home.join(".codex/ratatoskr-backups/test-review"),
            ownership: crate::engine::legacy_ownership(&settings, &legacy)?,
        };
        assert_eq!(preview.skill.files.len(), 2);
        assert!(legacy.exists());
        fs::write(legacy.join("SKILL.md"), "new edit")?;
        assert!(resolve_in(&settings, &serde_json::to_string(&preview)?, &home).is_err());
        preview.skill = duplicates_in(&settings, &home)?.remove(0);
        let valid = preview.backup.clone();
        preview.backup = root.join("outside");
        assert!(resolve_in(&settings, &serde_json::to_string(&preview)?, &home).is_err());
        preview.backup = valid;
        let backup = resolve_in(&settings, &serde_json::to_string(&preview)?, &home)?;
        assert!(!legacy.exists());
        let remaining: serde_json::Value = serde_json::from_slice(&fs::read(&manifest)?)?;
        assert!(remaining["files"].as_object().unwrap().is_empty());
        assert!(remaining["roots"].as_array().unwrap().is_empty());
        assert_eq!(fs::read_to_string(backup.join("SKILL.md"))?, "new edit");
        assert_eq!(fs::read(backup.join("__pycache__/keep.pyc"))?, [0, 1, 255]);
        assert!(backup.with_extension("json").is_file());
        assert!(duplicates_in(&settings, &home)?.is_empty());
        Ok(())
    }
}

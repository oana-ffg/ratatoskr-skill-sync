use super::*;

impl Fixture {
    fn targets(&self, mode: &str, harnesses: &[&str]) {
        let home = directories::BaseDirs::new()
            .unwrap()
            .home_dir()
            .to_string_lossy()
            .into_owned();
        let profile = toml::Table::from_iter([
            ("os".into(), std::env::consts::OS.into()),
            ("mode".into(), mode.into()),
            ("home".into(), home.into()),
            (
                "harnesses".into(),
                toml::Value::Array(harnesses.iter().map(|h| (*h).into()).collect()),
            ),
        ]);
        self.write("machines/test.toml", &toml::to_string(&profile).unwrap());
    }
    fn with_targets(mode: &str, harnesses: &[&str]) -> Self {
        let f = Self::new();
        let mut definitions = String::new();
        for name in ["alpha", "beta"] {
            let root = toml::Value::String(f.root.join(name).to_string_lossy().into_owned());
            let instructions = toml::Value::String(
                f.root
                    .join(format!("{name}-instructions"))
                    .to_string_lossy()
                    .into_owned(),
            );
            definitions.push_str(&format!("[{name}]\nskills_dir='skills'\nglobal_file='AGENTS.md'\nglobal_root={instructions}\n[{name}.roots]\nmacos={root}\nwindows={root}\nlinux={root}\n"));
        }
        f.write("harnesses.toml", &definitions);
        f.write(
            "skills/review-code/SKILL.md",
            "---\nname: review-code\ndescription: Review code\n---\nRead the diff.\n",
        );
        f.write("global/AGENTS.md", "Shared instructions\n");
        f.targets(mode, harnesses);
        f
    }
    fn resolve_stale(&self, action: ratatoskr_core::StaleAction) -> Vec<PathBuf> {
        let mut archives = vec![];
        for location in ratatoskr_core::stale_targets(&self.settings).unwrap() {
            let preview =
                ratatoskr_core::preview_stale_target(&self.settings, &location.path, action)
                    .unwrap();
            if let Some(archive) = ratatoskr_core::resolve_stale_target(
                &self.settings,
                &serde_json::to_string(&preview).unwrap(),
            )
            .unwrap()
            {
                archives.push(archive);
            }
        }
        archives
    }
}

#[test]
fn stale_live_target_blocks_apply_reviewed_and_sync_through_conflicts() {
    let mut f = Fixture::with_targets("live", &["alpha", "beta"]);
    f.settings.updates.automatic = false;
    let remote = f.root.join("remote");
    fs::create_dir(&remote).unwrap();
    git(&remote, &["init", "--bare", "-b", "main"]);
    git(&f.settings.source, &["init", "-b", "main"]);
    git(
        &f.settings.source,
        &["config", "user.name", "Ratatoskr tests"],
    );
    git(
        &f.settings.source,
        &["config", "user.email", "tests@localhost"],
    );
    git(&f.settings.source, &["config", "commit.gpgsign", "false"]);
    git(&f.settings.source, &["add", "."]);
    git(&f.settings.source, &["commit", "-m", "Initial"]);
    let remote_url = remote
        .to_str()
        .unwrap()
        .trim_start_matches("\\\\?\\")
        .replace('\\', "/");
    git(
        &f.settings.source,
        &["remote", "add", "origin", &remote_url],
    );
    git(&f.settings.source, &["push", "-u", "origin", "main"]);
    apply(&f.settings).unwrap();
    f.targets("live", &["alpha"]);
    git(&f.settings.source, &["add", "."]);
    git(&f.settings.source, &["commit", "-m", "Stop syncing beta"]);
    git(&f.settings.source, &["push"]);
    let report = plan(&f.settings).unwrap();
    assert_eq!(report.stale_targets.len(), 1);
    assert!(
        report
            .stale_targets
            .iter()
            .all(|l| l.harness.as_deref() == Some("beta"))
    );
    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(report.stale_targets[0].path, f.root.join("beta/skills"));
    assert_eq!(report.stale_targets[0].files.len(), 2);
    assert!(
        report.stale_targets[0]
            .files
            .iter()
            .any(|file| file.path == f.root.join("beta-instructions/AGENTS.md"))
    );
    assert!(report.changes.is_empty());
    assert!(apply(&f.settings).is_err());
    assert!(
        ratatoskr_core::apply_reviewed(&f.settings, &serde_json::to_string(&report).unwrap())
            .is_err()
    );
    let error = format!("{:#}", sync(&f.settings).unwrap_err());
    assert!(error.contains("Pending stale-target decision"), "{error}");
    assert!(f.root.join("beta/skills/review-code/SKILL.md").is_file());
    assert!(f.root.join("beta-instructions/AGENTS.md").is_file());
}

#[test]
fn stale_forget_preserves_files_and_reenabled_target_is_unmanaged_until_adopted() {
    let f = Fixture::with_targets("live", &["alpha", "beta"]);
    apply(&f.settings).unwrap();
    let skill = f.root.join("beta/skills/review-code/SKILL.md");
    let global = f.root.join("beta-instructions/AGENTS.md");
    let before = [fs::read(&skill).unwrap(), fs::read(&global).unwrap()];
    f.targets("live", &["alpha"]);
    assert!(
        f.resolve_stale(ratatoskr_core::StaleAction::Forget)
            .is_empty()
    );
    assert_eq!(
        [fs::read(&skill).unwrap(), fs::read(&global).unwrap()],
        before
    );
    let report = plan(&f.settings).unwrap();
    assert!(
        report.stale_targets.is_empty() && report.conflicts.is_empty() && report.changes.is_empty()
    );
    assert!(
        f.ownership()["target_harnesses"]
            .as_object()
            .unwrap()
            .values()
            .all(|name| name == "alpha")
    );
    f.targets("live", &["alpha", "beta"]);
    let report = plan(&f.settings).unwrap();
    assert!(
        report
            .conflict_details
            .iter()
            .any(|c| c.kind == "unmanaged-directory" && c.path == *skill.parent().unwrap())
    );
    assert!(
        report
            .conflicts
            .iter()
            .any(|c| c.contains(global.to_str().unwrap()))
    );
    let backup = ratatoskr_core::adopt(&f.settings).unwrap();
    let adopted = f.ownership()["target_harnesses"].clone();
    assert_eq!(
        adopted[skill.parent().unwrap().parent().unwrap().to_str().unwrap()],
        "beta"
    );
    assert_eq!(adopted[global.to_str().unwrap()], "beta");
    apply(&f.settings).unwrap();
    ratatoskr_core::restore_backup(&f.settings, &backup).unwrap();
    assert_eq!(f.ownership()["target_harnesses"], adopted);
    assert!(plan(&f.settings).unwrap().conflicts.is_empty());
}

#[test]
fn stale_remove_archives_edits_runtime_files_and_empty_directories_with_recovery_receipts() {
    let f = Fixture::with_targets("live", &["alpha", "beta"]);
    apply(&f.settings).unwrap();
    let skill_root = f.root.join("beta/skills/review-code");
    let skill = skill_root.join("SKILL.md");
    fs::write(&skill, "Hand edit with irreplaceable bytes").unwrap();
    fs::write(skill_root.join(".DS_Store"), b"Finder\0data").unwrap();
    fs::create_dir(skill_root.join("empty")).unwrap();
    fs::create_dir_all(f.root.join("beta/skills/unrelated")).unwrap();
    fs::write(
        f.root.join("beta/skills/unrelated/SKILL.md"),
        "Unmanaged skill",
    )
    .unwrap();
    f.targets("live", &["alpha"]);
    let locations = ratatoskr_core::stale_targets(&f.settings).unwrap();
    let location = locations.iter().find(|l| !l.roots.is_empty()).unwrap();
    assert!(
        location
            .files
            .iter()
            .any(|file| file.path == skill && file.modified)
    );
    assert!(
        location
            .files
            .iter()
            .any(|file| !file.owned && file.path.ends_with(".DS_Store"))
    );
    let archives = f.resolve_stale(ratatoskr_core::StaleAction::Remove);
    assert_eq!(archives.len(), 1);
    assert!(!skill_root.exists());
    assert!(!f.root.join("beta-instructions/AGENTS.md").exists());
    assert!(f.root.join("beta/skills/unrelated/SKILL.md").exists());
    let report = plan(&f.settings).unwrap();
    assert!(
        report.stale_targets.is_empty() && report.conflicts.is_empty() && report.changes.is_empty()
    );
    for archive in archives {
        let receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(archive.join("receipt.json")).unwrap()).unwrap();
        let archives = receipt["preview"]["archives"].as_array().unwrap();
        assert_eq!(archives.len(), 2);
        for directory in archives {
            let other: serde_json::Value = serde_json::from_slice(
                &fs::read(Path::new(directory.as_str().unwrap()).join("receipt.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(other, receipt);
        }
        for (original, stored) in receipt["moves"].as_object().unwrap() {
            let target = if Path::new(original).starts_with(&skill_root) {
                skill_root.parent().unwrap()
            } else {
                Path::new(original)
            };
            assert_eq!(
                Path::new(stored.as_str().unwrap())
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap(),
                target.parent().unwrap().join("ratatoskr-backups")
            );
            assert!(!Path::new(original).exists());
            fs::rename(stored.as_str().unwrap(), original).unwrap();
        }
    }
    assert_eq!(
        fs::read(&skill).unwrap(),
        b"Hand edit with irreplaceable bytes"
    );
    assert_eq!(
        fs::read(skill_root.join(".DS_Store")).unwrap(),
        b"Finder\0data"
    );
    assert!(skill_root.join("empty").is_dir());
    assert_eq!(
        fs::read(f.root.join("beta-instructions/AGENTS.md")).unwrap(),
        b"Shared instructions\n"
    );
}

#[test]
fn stale_preview_refuses_file_manifest_and_configuration_drift() {
    for drift in [
        "file",
        "manifest",
        "configuration",
        "runtime",
        "missing-root",
    ] {
        let f = Fixture::with_targets("live", &["alpha", "beta"]);
        apply(&f.settings).unwrap();
        f.targets("live", &["alpha"]);
        let location = f.root.join("beta/skills");
        let preview = ratatoskr_core::preview_stale_target(
            &f.settings,
            &location,
            ratatoskr_core::StaleAction::Remove,
        )
        .unwrap();
        let installed = location.join("review-code/SKILL.md");
        match drift {
            "file" => fs::write(&installed, "New edit").unwrap(),
            "runtime" => {
                fs::write(location.join("review-code/.DS_Store"), "new runtime file").unwrap()
            }
            "missing-root" => {
                fs::rename(location.join("review-code"), location.join("moved-by-user")).unwrap()
            }
            "manifest" => {
                f.resolve_stale(ratatoskr_core::StaleAction::Forget);
            }
            "configuration" => f.write(
                "harnesses.toml",
                &format!(
                    "{}\n# changed configuration\n",
                    fs::read_to_string(f.settings.source.join("harnesses.toml")).unwrap()
                ),
            ),
            _ => unreachable!(),
        }
        let error = format!(
            "{:#}",
            ratatoskr_core::resolve_stale_target(
                &f.settings,
                &serde_json::to_string(&preview).unwrap()
            )
            .unwrap_err()
        );
        assert!(
            error.contains("changed since preview; review again"),
            "{drift}: {error}"
        );
        assert!(!preview.archive.unwrap().exists());
    }
}

#[test]
fn staging_live_switches_require_explicit_decisions_in_both_directions() {
    for (from, to) in [("staging", "live"), ("live", "staging")] {
        let f = Fixture::with_targets(from, &["alpha"]);
        let initial = plan(&f.settings).unwrap();
        let old_paths: Vec<_> = initial.changes.iter().map(|c| c.path.clone()).collect();
        apply(&f.settings).unwrap();
        f.targets(to, &["alpha"]);
        let report = plan(&f.settings).unwrap();
        assert_eq!(report.stale_targets.len(), 1);
        assert!(apply(&f.settings).is_err());
        assert!(old_paths.iter().all(|p| p.is_file()));
        f.resolve_stale(ratatoskr_core::StaleAction::Remove);
        assert!(old_paths.iter().all(|p| !p.exists()));
        apply(&f.settings).unwrap();
        assert!(plan(&f.settings).unwrap().conflicts.is_empty());
    }
}

#[cfg(unix)]
#[test]
fn stale_permission_previews_detect_drift_and_remove_preserves_modes() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let f = Fixture::with_targets("live", &["alpha", "beta"]);
    apply(&f.settings).unwrap();
    let file = f.root.join("beta/skills/review-code/SKILL.md");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o700)).unwrap();
    f.targets("live", &["alpha"]);
    let preview = ratatoskr_core::preview_stale_target(
        &f.settings,
        &f.root.join("beta/skills"),
        ratatoskr_core::StaleAction::Remove,
    )
    .unwrap();
    assert!(
        preview
            .location
            .files
            .iter()
            .find(|f| f.path == file)
            .unwrap()
            .modified
    );
    fs::set_permissions(&file, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(
        ratatoskr_core::resolve_stale_target(
            &f.settings,
            &serde_json::to_string(&preview).unwrap()
        )
        .is_err()
    );
    let preview = ratatoskr_core::preview_stale_target(
        &f.settings,
        &f.root.join("beta/skills"),
        ratatoskr_core::StaleAction::Remove,
    )
    .unwrap();
    let archive = ratatoskr_core::resolve_stale_target(
        &f.settings,
        &serde_json::to_string(&preview).unwrap(),
    )
    .unwrap()
    .unwrap();
    let archived = archive.join("entries/0/SKILL.md");
    assert_eq!(fs::metadata(archived).unwrap().mode() & 0o777, 0o750);
}

#[test]
fn changed_roots_and_instruction_paths_keep_recorded_harness_labels() {
    for change in ["root", "skills", "instructions"] {
        let f = Fixture::with_targets("live", &["alpha"]);
        apply(&f.settings).unwrap();
        let definitions = fs::read_to_string(f.settings.source.join("harnesses.toml")).unwrap();
        let changed = match change {
            "root" => definitions.replace(
                &toml::Value::String(f.root.join("alpha").to_string_lossy().into_owned())
                    .to_string(),
                &toml::Value::String(
                    f.root
                        .join("relocated-alpha")
                        .to_string_lossy()
                        .into_owned(),
                )
                .to_string(),
            ),
            "skills" => definitions.replace("skills_dir='skills'", "skills_dir='custom-skills'"),
            "instructions" => {
                definitions.replace("global_file='AGENTS.md'", "global_file='NEW.md'")
            }
            _ => unreachable!(),
        };
        assert_ne!(changed, definitions);
        f.write("harnesses.toml", &changed);
        let report = plan(&f.settings).unwrap();
        assert_eq!(report.stale_targets.len(), 1, "{change}");
        assert_eq!(
            report.stale_targets[0].harness.as_deref(),
            Some("alpha"),
            "{change}"
        );
        f.resolve_stale(ratatoskr_core::StaleAction::Remove);
        apply(&f.settings).unwrap();
        assert!(plan(&f.settings).unwrap().conflicts.is_empty(), "{change}");
    }
}

#[test]
fn empty_owned_roots_and_missing_owned_files_still_require_a_decision() {
    let f = Fixture::with_targets("live", &["alpha", "beta"]);
    apply(&f.settings).unwrap();
    let file = f.root.join("beta/skills/review-code/SKILL.md");
    fs::remove_file(&file).unwrap();
    f.targets("live", &["alpha"]);
    let locations = ratatoskr_core::stale_targets(&f.settings).unwrap();
    assert_eq!(locations.len(), 1);
    assert!(
        locations
            .iter()
            .flat_map(|l| &l.files)
            .any(|f| f.path == file && f.modified)
    );
    f.resolve_stale(ratatoskr_core::StaleAction::Remove);
    assert!(!file.parent().unwrap().exists());
    assert!(plan(&f.settings).unwrap().conflicts.is_empty());
}

#[cfg(unix)]
#[test]
fn failed_stale_manifest_write_rolls_back_archived_files() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::with_targets("live", &["alpha", "beta"]);
    apply(&f.settings).unwrap();
    f.targets("live", &["alpha"]);
    let path = f.root.join("beta/skills");
    let preview = ratatoskr_core::preview_stale_target(
        &f.settings,
        &path,
        ratatoskr_core::StaleAction::Remove,
    )
    .unwrap();
    let state = f.settings.state_dir.join("test");
    let manifest = state.join("manifest.json");
    let before = fs::read(&manifest).unwrap();
    let skill = path.join("review-code/SKILL.md");
    let bytes = fs::read(&skill).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o500)).unwrap();
    let result = ratatoskr_core::resolve_stale_target(
        &f.settings,
        &serde_json::to_string(&preview).unwrap(),
    );
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    let error = format!("{:#}", result.unwrap_err());
    assert!(
        error.contains("moves rolled back") && error.contains("ownership unchanged"),
        "{error}"
    );
    assert_eq!(fs::read(&skill).unwrap(), bytes);
    assert_eq!(fs::read(&manifest).unwrap(), before);
    assert!(
        error.contains(
            preview
                .archive
                .as_ref()
                .unwrap()
                .join("receipt.json")
                .to_str()
                .unwrap()
        ),
        "{error}"
    );
    assert!(!error.contains("Some("), "{error}");
    assert!(preview.archive.unwrap().join("receipt.json").is_file());
}

#[test]
fn redirected_or_occupied_stale_archives_leave_ownership_and_files_unchanged() {
    for redirect in [false, true] {
        let f = Fixture::with_targets("live", &["alpha", "beta"]);
        apply(&f.settings).unwrap();
        f.targets("live", &["alpha"]);
        let preview = ratatoskr_core::preview_stale_target(
            &f.settings,
            &f.root.join("beta/skills"),
            ratatoskr_core::StaleAction::Remove,
        )
        .unwrap();
        let mut value = serde_json::to_value(&preview).unwrap();
        if redirect {
            value["archive"] = f
                .root
                .join("redirected")
                .to_string_lossy()
                .into_owned()
                .into();
        } else {
            fs::create_dir_all(preview.archive.as_ref().unwrap()).unwrap();
        }
        let manifest = f.settings.state_dir.join("test/manifest.json");
        let before = fs::read(&manifest).unwrap();
        assert!(
            ratatoskr_core::resolve_stale_target(
                &f.settings,
                &serde_json::to_string(&value).unwrap()
            )
            .is_err()
        );
        assert_eq!(fs::read(&manifest).unwrap(), before);
        assert!(f.root.join("beta/skills/review-code/SKILL.md").is_file());
    }
}

#[cfg(unix)]
#[test]
fn stale_symlinks_remain_hard_errors_even_for_forget() {
    let f = Fixture::with_targets("live", &["alpha", "beta"]);
    apply(&f.settings).unwrap();
    f.targets("live", &["alpha"]);
    let root = f.root.join("beta/skills/review-code");
    std::os::unix::fs::symlink(root.join("SKILL.md"), root.join(".DS_Store")).unwrap();
    assert!(plan(&f.settings).is_err());
    assert!(
        ratatoskr_core::preview_stale_target(
            &f.settings,
            root.parent().unwrap(),
            ratatoskr_core::StaleAction::Forget
        )
        .is_err()
    );
}

#[test]
fn stale_decisions_precede_other_conflict_backups_so_restore_stays_usable() {
    let f = Fixture::with_targets("live", &["alpha", "beta"]);
    apply(&f.settings).unwrap();
    let active = f.root.join("alpha/skills/review-code/SKILL.md");
    fs::write(&active, "Current target's hand edit").unwrap();
    f.targets("live", &["alpha"]);
    let error = format!(
        "{:#}",
        ratatoskr_core::preview_resolution(
            &f.settings,
            &active,
            ratatoskr_core::ConflictAction::Reinstall
        )
        .err()
        .unwrap()
    );
    assert!(
        error.contains("Resolve pending stale-target decisions"),
        "{error}"
    );
    assert!(ratatoskr_core::adopt(&f.settings).is_err());
    assert!(!f.settings.state_dir.join("test/backups").exists());
    assert_eq!(fs::read(&active).unwrap(), b"Current target's hand edit");
    f.resolve_stale(ratatoskr_core::StaleAction::Forget);
    let preview = ratatoskr_core::preview_resolution(
        &f.settings,
        &active,
        ratatoskr_core::ConflictAction::Reinstall,
    )
    .unwrap();
    let backup =
        ratatoskr_core::resolve_conflict(&f.settings, &serde_json::to_string(&preview).unwrap())
            .unwrap();
    ratatoskr_core::restore_backup(&f.settings, &backup).unwrap();
    assert_eq!(fs::read(&active).unwrap(), b"Current target's hand edit");
}

#[test]
fn stale_remove_leaves_contents_of_overlapping_current_targets_in_place() {
    let f = Fixture::with_targets("live", &["alpha"]);
    apply(&f.settings).unwrap();
    let old_file = f.root.join("alpha/skills/review-code/SKILL.md");
    let definitions = fs::read_to_string(f.settings.source.join("harnesses.toml")).unwrap();
    f.write(
        "harnesses.toml",
        &definitions.replace(
            "skills_dir='skills'",
            "skills_dir='skills/review-code/current'",
        ),
    );
    let current_file = f
        .root
        .join("alpha/skills/review-code/current/review-code/SKILL.md");
    fs::create_dir_all(current_file.parent().unwrap()).unwrap();
    fs::write(&current_file, "New target's existing copy").unwrap();
    let preview = ratatoskr_core::preview_stale_target(
        &f.settings,
        &f.root.join("alpha/skills"),
        ratatoskr_core::StaleAction::Remove,
    )
    .unwrap();
    let value = serde_json::to_value(&preview).unwrap();
    assert!(
        value["location"]["archive_roots"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let archive = ratatoskr_core::resolve_stale_target(
        &f.settings,
        &serde_json::to_string(&preview).unwrap(),
    )
    .unwrap()
    .unwrap();
    assert!(!old_file.exists());
    assert_eq!(
        fs::read(&current_file).unwrap(),
        b"New target's existing copy"
    );
    assert_eq!(
        fs::read(archive.join("entries/0")).unwrap(),
        fs::read(f.settings.source.join("skills/review-code/SKILL.md")).unwrap()
    );
    assert!(plan(&f.settings).unwrap().stale_targets.is_empty());
}

#[test]
fn stale_harness_labels_survive_removed_definitions() {
    let f = Fixture::with_targets("live", &["alpha", "beta"]);
    apply(&f.settings).unwrap();
    f.targets("live", &["alpha"]);
    let definitions = fs::read_to_string(f.settings.source.join("harnesses.toml")).unwrap();
    f.write(
        "harnesses.toml",
        definitions.split("[beta]").next().unwrap(),
    );
    let locations = ratatoskr_core::stale_targets(&f.settings).unwrap();
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].harness.as_deref(), Some("beta"));
    assert_eq!(locations[0].path, f.root.join("beta/skills"));
    assert_eq!(locations[0].files.len(), 2);
    f.resolve_stale(ratatoskr_core::StaleAction::Remove);
    assert!(plan(&f.settings).unwrap().conflicts.is_empty());
}

#[test]
fn old_manifests_keep_path_only_decisions_until_applied_again() {
    let f = Fixture::with_targets("live", &["alpha", "beta"]);
    apply(&f.settings).unwrap();
    let mut old = f.ownership();
    old.as_object_mut().unwrap().remove("target_harnesses");
    fs::write(
        f.settings.state_dir.join("test/manifest.json"),
        serde_json::to_vec(&old).unwrap(),
    )
    .unwrap();
    f.targets("live", &["alpha"]);
    let locations = ratatoskr_core::stale_targets(&f.settings).unwrap();
    assert_eq!(locations.len(), 2);
    assert!(locations.iter().all(|location| location.harness.is_none()));
    assert!(
        locations
            .iter()
            .any(|location| location.path == f.root.join("beta/skills"))
    );
    assert!(
        locations
            .iter()
            .any(|location| location.path == f.root.join("beta-instructions/AGENTS.md"))
    );
    f.resolve_stale(ratatoskr_core::StaleAction::Forget);
    apply(&f.settings).unwrap();
    assert_eq!(
        f.ownership()["target_harnesses"][f.root.join("alpha/skills").to_str().unwrap()],
        "alpha"
    );
}

#[test]
fn global_only_stale_harness_uses_the_instructions_path() {
    let f = Fixture::with_targets("live", &["alpha", "beta"]);
    f.write(
        "skills/review-code/skill.toml",
        "[target]\nharnesses=['alpha']\n",
    );
    apply(&f.settings).unwrap();
    f.targets("live", &["alpha"]);
    let locations = ratatoskr_core::stale_targets(&f.settings).unwrap();
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].harness.as_deref(), Some("beta"));
    assert_eq!(
        locations[0].path,
        f.root.join("beta-instructions/AGENTS.md")
    );
    assert_eq!(locations[0].files.len(), 1);
    f.resolve_stale(ratatoskr_core::StaleAction::Remove);
    assert!(plan(&f.settings).unwrap().conflicts.is_empty());
}

#[test]
fn stale_harness_groups_global_and_project_targets_in_one_decision() {
    let mut f = Fixture::with_targets("live", &["alpha", "beta"]);
    let project = f.root.join("project");
    fs::create_dir_all(project.join("agent-skills/skills/project-review")).unwrap();
    git(&project, &["init", "-b", "main"]);
    fs::write(
        project.join("agent-skills/skills/project-review/skill.toml"),
        "",
    )
    .unwrap();
    fs::write(
        project.join("agent-skills/skills/project-review/SKILL.md"),
        "---\nname: project-review\ndescription: Review project\n---\nRead the diff.\n",
    )
    .unwrap();
    let config = f.root.join("config.toml");
    save(&config, &f.settings).unwrap();
    add_project(&config, &project, "agent-skills").unwrap();
    f.settings = load(&config).unwrap();
    apply(&f.settings).unwrap();
    f.targets("live", &["alpha"]);
    let locations = ratatoskr_core::stale_targets(&f.settings).unwrap();
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].harness.as_deref(), Some("beta"));
    assert_eq!(locations[0].roots.len(), 2);
    assert_eq!(locations[0].files.len(), 3);
    assert_eq!(locations[0].path, f.root.join("beta/skills"));
    let preview = ratatoskr_core::preview_stale_target(
        &f.settings,
        &locations[0].path,
        ratatoskr_core::StaleAction::Remove,
    )
    .unwrap();
    assert_eq!(preview.archives.len(), 3);
    let archives = f.resolve_stale(ratatoskr_core::StaleAction::Remove);
    assert_eq!(archives.len(), 1);
    assert!(!project.join(".beta/skills/project-review").exists());
    assert!(!f.root.join("beta/skills/review-code").exists());
    assert!(!f.root.join("beta-instructions/AGENTS.md").exists());
    assert!(plan(&f.settings).unwrap().conflicts.is_empty());
}

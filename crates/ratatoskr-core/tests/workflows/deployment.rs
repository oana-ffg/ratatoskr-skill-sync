use super::*;

#[cfg(windows)]
#[test]
fn windows_canonical_prefix_cannot_hide_source_state_overlap() {
    let f = Fixture::new();
    let mut settings = f.settings.clone();
    let ordinary = settings
        .source
        .to_str()
        .unwrap()
        .trim_start_matches("\\\\?\\");
    settings.state_dir = PathBuf::from(ordinary).join("state");
    assert!(settings.validate().is_err());
}

#[test]
fn apply_returns_the_counts_written_under_the_lock() {
    use ratatoskr_core::AppliedChanges;
    let f = Fixture::new();
    let planned = plan(&f.settings).unwrap();
    assert!(planned.applied.is_none());
    let expected = AppliedChanges {
        added: planned.changes.len(),
        updated: 0,
        removed: 0,
    };
    assert!(expected.added > 0);
    let written = apply(&f.settings).unwrap();
    assert!(written.changes.is_empty());
    assert_eq!(written.applied, Some(expected));
    f.write("skills/review-code/guide.txt", "Updated guidance");
    let updated = apply(&f.settings).unwrap();
    assert_eq!(
        updated.applied,
        Some(AppliedChanges {
            added: 1,
            updated: 0,
            removed: 0,
        })
    );
    f.write("skills/review-code/guide.txt", "Revised guidance");
    let revised = apply(&f.settings).unwrap();
    assert_eq!(
        revised.applied,
        Some(AppliedChanges {
            added: 0,
            updated: 1,
            removed: 0,
        })
    );
    fs::remove_file(f.settings.source.join("skills/review-code/guide.txt")).unwrap();
    let removed = apply(&f.settings).unwrap();
    assert_eq!(
        removed.applied,
        Some(AppliedChanges {
            added: 0,
            updated: 0,
            removed: 1,
        })
    );
    let unchanged = apply(&f.settings).unwrap();
    assert_eq!(unchanged.applied, Some(AppliedChanges::default()));
    assert!(plan(&f.settings).unwrap().applied.is_none());
}

#[test]
fn renders_applies_and_updates_without_rewriting_identical_files() {
    let f = Fixture::new();
    let initial = plan(&f.settings).unwrap();
    assert_eq!(initial.changes.len(), 1);
    assert!(
        initial.changes[0]
            .diff
            .as_ref()
            .unwrap()
            .contains("Read /knowledge")
    );
    assert!(
        !f.settings.state_dir.exists(),
        "Preview must not create state"
    );
    assert!(apply(&f.settings).unwrap().changes.is_empty());
    let output = fs::read_to_string(f.output().join("SKILL.md")).unwrap();
    assert!(output.contains("Report findings."));
    assert!(!output.contains("af:"));
    let modified = fs::metadata(f.output().join("SKILL.md"))
        .unwrap()
        .modified()
        .unwrap();
    apply(&f.settings).unwrap();
    assert_eq!(
        fs::metadata(f.output().join("SKILL.md"))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    f.write("skills/review-code/guide.txt", "Updated guidance");
    assert_eq!(plan(&f.settings).unwrap().changes.len(), 1);
    apply(&f.settings).unwrap();
    assert_eq!(
        fs::read_to_string(f.output().join("guide.txt")).unwrap(),
        "Updated guidance"
    );
}

#[test]
fn hand_edit_refuses_entire_apply_including_unrelated_additions() {
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    fs::write(f.output().join("SKILL.md"), "User's edit").unwrap();
    f.write("skills/review-code/new.txt", "New file");
    assert!(!plan(&f.settings).unwrap().conflicts.is_empty());
    assert!(apply(&f.settings).is_err());
    assert!(!f.output().join("new.txt").exists());
    assert_eq!(
        fs::read_to_string(f.output().join("SKILL.md")).unwrap(),
        "User's edit"
    );
}

#[test]
fn existing_directory_is_not_implicitly_adopted_even_if_empty() {
    let f = Fixture::new();
    fs::create_dir_all(f.output()).unwrap();
    assert!(apply(&f.settings).is_err());
    assert!(!f.output().join("SKILL.md").exists());
}

#[test]
fn missing_owned_and_new_unmanaged_files_are_conflicts() {
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    fs::remove_file(f.output().join("SKILL.md")).unwrap();
    assert!(apply(&f.settings).is_err());
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    fs::write(f.output().join("personal.txt"), "Keep this").unwrap();
    assert!(apply(&f.settings).is_err());
    assert_eq!(
        fs::read_to_string(f.output().join("personal.txt")).unwrap(),
        "Keep this"
    );
}

#[test]
fn removing_a_source_deletes_only_owned_files() {
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    let unrelated = f.output().parent().unwrap().join("other-skill");
    fs::create_dir_all(&unrelated).unwrap();
    fs::write(unrelated.join("SKILL.md"), "Unrelated").unwrap();
    fs::remove_dir_all(f.settings.source.join("skills/review-code")).unwrap();
    apply(&f.settings).unwrap();
    assert!(!f.output().join("SKILL.md").exists());
    assert!(unrelated.join("SKILL.md").exists());
    f.write("skills/review-code/skill.toml", "");
    f.write(
        "skills/review-code/SKILL.md",
        "---\nname: review-code\ndescription: Review code\n---\nRead the changes.\n",
    );
    apply(&f.settings).unwrap();
    assert!(
        f.output().join("SKILL.md").exists(),
        "A removed skill can be reinstalled"
    );
}

#[test]
fn preview_hashes_change_for_binary_files_even_without_a_text_diff() {
    let f = Fixture::new();
    fs::write(
        f.settings.source.join("skills/review-code/asset.bin"),
        [0xff, 0xfe],
    )
    .unwrap();
    let before = plan(&f.settings).unwrap();
    fs::write(
        f.settings.source.join("skills/review-code/asset.bin"),
        [0xff, 0xfd],
    )
    .unwrap();
    let after = plan(&f.settings).unwrap();
    assert!(before.changes[1].diff.is_none());
    assert_ne!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
}

#[test]
fn unsafe_paths_and_overlapping_state_are_rejected() {
    let mut f = Fixture::new();
    f.settings.machine = "../outside".into();
    assert!(apply(&f.settings).is_err());
    f.settings.machine = "test".into();
    f.settings.state_dir = f.settings.source.join("state");
    assert!(apply(&f.settings).is_err());
    let f = Fixture::new();
    f.write("harnesses.toml", "[codex]\nskills_dir='../escape'\nglobal_file='AGENTS.md'\n[codex.roots]\nmacos='/agents'\n");
    assert!(plan(&f.settings).is_err());
}

#[test]
fn ownership_manifest_cannot_point_outside_configured_targets() {
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    let manifest = f.settings.state_dir.join("test/manifest.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["files"][f.root.join("outside.txt").to_str().unwrap()] =
        serde_json::json!({"sha256":"abc", "executable":false});
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(apply(&f.settings).is_err());
}

#[cfg(unix)]
#[test]
fn source_and_destination_symlinks_are_refused() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    symlink(
        "/etc/hosts",
        f.settings.source.join("skills/review-code/secret.txt"),
    )
    .unwrap();
    assert!(plan(&f.settings).is_err());
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    fs::remove_file(f.output().join("SKILL.md")).unwrap();
    symlink("/etc/hosts", f.output().join("SKILL.md")).unwrap();
    assert!(apply(&f.settings).is_err());
}

#[cfg(unix)]
#[test]
fn executable_bits_are_preserved_and_drift_is_detected() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.write("skills/review-code/run.sh", "#!/bin/sh\nexit 0\n");
    fs::set_permissions(
        f.settings.source.join("skills/review-code/run.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    apply(&f.settings).unwrap();
    assert_ne!(
        fs::metadata(f.output().join("run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    fs::set_permissions(f.output().join("run.sh"), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(apply(&f.settings).is_err());
}

#[test]
fn adoption_backs_up_originals_and_restore_recovers_removed_files() {
    let f = Fixture::new();
    fs::create_dir_all(f.output()).unwrap();
    fs::write(f.output().join("SKILL.md"), "Original hand-authored skill").unwrap();
    fs::write(f.output().join("extra.txt"), "Preserve in backup").unwrap();
    let backup = ratatoskr_core::adopt(&f.settings).unwrap();
    assert_eq!(
        fs::read_to_string(f.output().join("SKILL.md")).unwrap(),
        "Original hand-authored skill"
    );
    apply(&f.settings).unwrap();
    assert!(!f.output().join("extra.txt").exists());
    ratatoskr_core::restore_backup(&f.settings, &backup).unwrap();
    assert_eq!(
        fs::read_to_string(f.output().join("SKILL.md")).unwrap(),
        "Original hand-authored skill"
    );
    assert_eq!(
        fs::read_to_string(f.output().join("extra.txt")).unwrap(),
        "Preserve in backup"
    );
    fs::write(f.output().join("SKILL.md"), "Another manual edit").unwrap();
    assert!(ratatoskr_core::adopt(&f.settings).is_err());
    assert!(ratatoskr_core::restore_backup(&f.settings, &backup).is_err());
}

#[test]
fn reviewed_plan_refuses_changed_source_before_writing() {
    let f = Fixture::new();
    let preview = serde_json::to_string(&plan(&f.settings).unwrap()).unwrap();
    f.write("skills/review-code/extra.txt", "New source after review");
    assert!(ratatoskr_core::apply_reviewed(&f.settings, &preview).is_err());
    assert!(!f.output().exists());
}

#[test]
fn adoption_preserves_global_instructions_when_no_global_source_exists() {
    let f = Fixture::new();
    let global = f.settings.state_dir.join("test/staging/codex/AGENTS.md");
    fs::create_dir_all(global.parent().unwrap()).unwrap();
    fs::write(&global, "Existing instructions outside the content source").unwrap();
    ratatoskr_core::adopt(&f.settings).unwrap();
    let preview = plan(&f.settings).unwrap();
    assert!(!preview.changes.iter().any(|change| change.path == global));
    apply(&f.settings).unwrap();
    assert_eq!(
        fs::read_to_string(global).unwrap(),
        "Existing instructions outside the content source"
    );
}

#[test]
fn dropping_an_applied_agent_explains_the_stale_ownership() {
    let f = Fixture::new();
    f.write(
        "skills/review-code/SKILL.md",
        "---\nname: review-code\ndescription: Review changed code\n---\nRead the diff.\n",
    );
    let harnesses = fs::read_to_string(f.settings.source.join("harnesses.toml")).unwrap();
    f.write(
        "harnesses.toml",
        &format!("{harnesses}[claude]\nskills_dir='skills'\nglobal_file='CLAUDE.md'\n[claude.roots]\nmacos='{{{{af:home}}}}/.claude'\nwindows='{{{{af:home}}}}/.claude'\nlinux='{{{{af:home}}}}/.claude'\n"),
    );
    let profile = "os='macos'\nmode='staging'\ncapabilities=[]\n[paths]\nvault='/knowledge'\n";
    f.write(
        "machines/test.toml",
        &format!("harnesses=['codex','claude']\n{profile}"),
    );
    apply(&f.settings).unwrap();
    f.write(
        "machines/test.toml",
        &format!("harnesses=['codex']\n{profile}"),
    );
    let report = plan(&f.settings).unwrap();
    assert_eq!(report.stale_targets.len(), 1);
    assert_eq!(report.stale_targets[0].harness.as_deref(), Some("claude"));
    assert!(report.conflicts[0].contains("ratatoskr stale-target"));
    assert!(report.changes.is_empty());
    assert!(apply(&f.settings).is_err());
    assert!(
        ratatoskr_core::apply_reviewed(&f.settings, &serde_json::to_string(&report).unwrap())
            .is_err()
    );
}

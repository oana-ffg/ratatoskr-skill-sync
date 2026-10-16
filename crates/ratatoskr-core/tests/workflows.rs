use ratatoskr_core::{Settings, add_project, apply, load, plan, project_status, save, sync};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    settings: Settings,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = root.join("content");
        fs::create_dir_all(source.join("machines")).unwrap();
        fs::create_dir_all(source.join("skills/review-code")).unwrap();
        fs::write(source.join("harnesses.toml"), "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\n[codex.roots]\nmacos='{{af:home}}/.codex'\nwindows='{{af:home}}/.codex'\nlinux='{{af:home}}/.codex'\n").unwrap();
        fs::write(source.join("machines/test.toml"), "os='macos'\nmode='staging'\nharnesses=['codex']\ncapabilities=[]\n[paths]\nvault='/knowledge'\n").unwrap();
        fs::write(source.join("skills/review-code/skill.toml"), "").unwrap();
        fs::write(source.join("skills/review-code/SKILL.md"), "---\nname: review-code\ndescription: Review changed code\n---\nRead {{af:paths.vault}}.\n<!-- only harness=codex -->Report findings.<!-- /only -->\n<!-- only harness=claude -->{{af:missing}}<!-- /only -->\n").unwrap();
        let settings = Settings {
            source,
            machine: "test".into(),
            state_dir: root.join("state"),
            projects: vec![],
            ..Settings::default()
        };
        Self {
            _temp: temp,
            root,
            settings,
        }
    }
    fn output(&self) -> PathBuf {
        self.settings
            .state_dir
            .join("test/staging/codex/skills/review-code")
    }
    fn write(&self, path: &str, content: &str) {
        let path = self.settings.source.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
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
fn variants_and_harness_frontmatter_are_rendered() {
    let f = Fixture::new();
    f.write(
        "skills/review-code/variants/harness=codex/guide.txt",
        "For {{af:harness}}",
    );
    f.write(
        "skills/review-code/skill.toml",
        "[frontmatter.codex]\nlicense='MIT'\n",
    );
    apply(&f.settings).unwrap();
    assert_eq!(
        fs::read_to_string(f.output().join("guide.txt")).unwrap(),
        "For codex"
    );
    assert!(
        fs::read_to_string(f.output().join("SKILL.md"))
            .unwrap()
            .contains("license: MIT")
    );
}

#[test]
fn colliding_variants_and_identity_overrides_are_rejected() {
    let f = Fixture::new();
    f.write(
        "skills/review-code/variants/harness=codex/guide.txt",
        "Harness",
    );
    f.write("skills/review-code/variants/os=macos/guide.txt", "OS");
    assert!(plan(&f.settings).is_err());
    let f = Fixture::new();
    f.write(
        "skills/review-code/skill.toml",
        "[frontmatter.codex]\nname='different'\n",
    );
    assert!(plan(&f.settings).is_err());
}

#[test]
fn capabilities_exclude_skills_and_unknown_selectors_fail() {
    let f = Fixture::new();
    f.write(
        "skills/review-code/skill.toml",
        "[target]\ncapabilities=['gopass']\n",
    );
    assert_eq!(plan(&f.settings).unwrap().skills, 0);
    f.write(
        "skills/review-code/skill.toml",
        "[target]\nplatfrom=['macos']\n",
    );
    assert!(plan(&f.settings).is_err());
}

#[test]
fn malformed_templates_and_yaml_fail_before_writes() {
    let f = Fixture::new();
    for body in [
        "<!-- only harness=codex -->Oops",
        "<!-- only harness=claude --><!-- only os=macos -->nested<!-- /only --><!-- /only -->",
        "{{af:unknown}}",
        "{{af:home",
        "<!-- only harness=codex harness=claude -->x<!-- /only -->",
    ] {
        f.write("skills/review-code/guide.md", body);
        assert!(plan(&f.settings).is_err(), "Unexpectedly accepted {body}");
        assert!(!f.output().exists());
    }
    f.write("skills/review-code/guide.md", "Fine");
    f.write(
        "skills/review-code/SKILL.md",
        "---\nname: review-code\nname: duplicate\ndescription: Test\n---\nBody\n",
    );
    assert!(plan(&f.settings).is_err());
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

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn project_monitor_counts_only_skill_changes_and_surfaces_missing_upstream() {
    let f = Fixture::new();
    let project = f.root.join("project");
    fs::create_dir_all(project.join("agent-skills")).unwrap();
    git(&project, &["init", "-b", "main"]);
    fs::write(project.join("agent-skills/SKILL.md"), "Skill source").unwrap();
    fs::write(project.join("unrelated.txt"), "Not a skill").unwrap();
    let config = f.root.join("config.toml");
    save(&config, &f.settings).unwrap();
    let status = add_project(&config, &project, "agent-skills").unwrap();
    assert_eq!(status.changed_files, 2); // Source plus project marker, not unrelated.txt.
    assert!(status.upstream.is_none());
    assert!(status.outgoing_commits.is_none());
    assert_eq!(load(&config).unwrap().projects, vec![project.clone()]);
    fs::remove_file(project.join(".ratatoskr.toml")).unwrap();
    assert!(project_status(&project).error.is_some());
}

#[test]
fn project_monitor_detects_only_unpushed_skill_commits() {
    let f = Fixture::new();
    let remote = f.root.join("remote");
    fs::create_dir(&remote).unwrap();
    git(&remote, &["init", "--bare", "-b", "main"]);
    let project = f.root.join("project");
    fs::create_dir_all(project.join("agent-skills")).unwrap();
    git(&project, &["init", "-b", "main"]);
    git(&project, &["config", "user.name", "Ratatoskr tests"]);
    git(&project, &["config", "user.email", "tests@localhost"]);
    git(&project, &["config", "commit.gpgsign", "false"]);
    fs::write(project.join("agent-skills/SKILL.md"), "Initial").unwrap();
    fs::write(project.join(".ratatoskr.toml"), "source='agent-skills'\n").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-m", "Initial"]);
    git(
        &project,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&project, &["push", "-u", "origin", "main"]);
    fs::write(project.join("unrelated.txt"), "Change").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-m", "Unrelated"]);
    assert_eq!(project_status(&project).outgoing_commits, Some(0));
    fs::write(project.join("agent-skills/SKILL.md"), "Changed").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-m", "Skills"]);
    assert_eq!(project_status(&project).outgoing_commits, Some(1));
}

#[test]
fn sync_refuses_dirty_content_without_modifying_it() {
    let f = Fixture::new();
    git(&f.settings.source, &["init", "-b", "main"]);
    assert!(sync(&f.settings).is_err());
    assert!(!f.settings.state_dir.join("test/staging").exists());
}

#[test]
fn source_editor_validates_before_save_and_preserves_concurrent_edits() {
    let f = Fixture::new();
    let original = ratatoskr_core::read_skill(&f.settings, None, "review-code").unwrap();
    assert!(
        ratatoskr_core::write_skill_file(
            &f.settings,
            None,
            "review-code",
            "SKILL.md",
            "Broken",
            &original.markdown_hash
        )
        .is_err()
    );
    assert_eq!(
        ratatoskr_core::read_skill(&f.settings, None, "review-code")
            .unwrap()
            .markdown,
        original.markdown
    );
    let revised = original
        .markdown
        .replace("Report findings.", "Report prioritized findings.");
    ratatoskr_core::write_skill_file(
        &f.settings,
        None,
        "review-code",
        "SKILL.md",
        &revised,
        &original.markdown_hash,
    )
    .unwrap();
    assert!(
        ratatoskr_core::write_skill_file(
            &f.settings,
            None,
            "review-code",
            "SKILL.md",
            &original.markdown,
            &original.markdown_hash
        )
        .is_err()
    );
    ratatoskr_core::disable_skill(&f.settings, None, "review-code", true).unwrap();
    assert_eq!(plan(&f.settings).unwrap().skills, 0);
    ratatoskr_core::disable_skill(&f.settings, None, "review-code", false).unwrap();
    assert_eq!(plan(&f.settings).unwrap().skills, 1);
    let archived = ratatoskr_core::archive_skill(&f.settings, None, "review-code").unwrap();
    assert!(archived.join("SKILL.md").is_file());
    assert!(!f.settings.source.join("skills/review-code").exists());
    ratatoskr_core::create_skill(&f.settings, None, "review-code", &original.markdown).unwrap();
    assert_eq!(plan(&f.settings).unwrap().skills, 1);
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
fn project_content_renders_with_shared_machine_settings_and_can_be_detached() {
    let f = Fixture::new();
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
        "---\nname: project-review\ndescription: Review this project\n---\nUse {{af:machine}}.\n",
    )
    .unwrap();
    let config = f.root.join("config.toml");
    save(&config, &f.settings).unwrap();
    add_project(&config, &project, "agent-skills").unwrap();
    let settings = load(&config).unwrap();
    let initial = plan(&settings).unwrap();
    assert_eq!(initial.skills, 2);
    let output = initial
        .changes
        .iter()
        .find(|c| c.path.ends_with("project-review/SKILL.md"))
        .unwrap()
        .path
        .clone();
    apply(&settings).unwrap();
    assert!(fs::read_to_string(&output).unwrap().contains("Use test."));
    ratatoskr_core::remove_project(&config, &project).unwrap();
    assert!(output.is_file(), "Detaching preserves project output");
    assert!(plan(&load(&config).unwrap()).unwrap().conflicts.is_empty());
}

#[test]
fn schedule_preview_escapes_paths_and_does_not_install_anything() {
    if !matches!(std::env::consts::OS, "macos" | "windows") {
        return;
    }
    let f = Fixture::new();
    let helper = f.root.join("Ratatoskr & helper");
    fs::write(&helper, "Test helper").unwrap();
    let config = f.root.join("configuration & data.toml");
    save(&config, &f.settings).unwrap();
    let schedule = ratatoskr_core::schedule_plan(&f.settings, &config, &helper, 60).unwrap();
    assert!(schedule.definition.contains("&amp;"));
    assert!(!schedule.definition.contains("&amp;amp;"));
    assert!(!f.settings.state_dir.exists());
    assert!(ratatoskr_core::schedule_plan(&f.settings, &config, &helper, 1).is_err());
}

#[test]
fn content_setup_creates_real_host_profile_and_stays_in_staging() {
    let f = Fixture::new();
    let root = f.root.join("new-content");
    let config = f.root.join("new-config.toml");
    let settings = ratatoskr_core::create_content(&config, &root, "draft").unwrap();
    assert_eq!(plan(&settings).unwrap().mode, "staging");
    assert_eq!(plan(&settings).unwrap().targets, 2);
    assert!(root.join(".git").is_dir());
    assert!(ratatoskr_core::create_content(&config, &root, "draft").is_err());
}

#[test]
fn signed_updates_accept_tauri_signatures_and_reject_tampering() {
    let key = include_str!("fixtures/update.pub");
    let payload = include_bytes!("fixtures/update.txt");
    let signature = include_str!("fixtures/update.txt.sig");
    ratatoskr_core::updates::verify(key, payload, signature).unwrap();
    assert!(ratatoskr_core::updates::verify(key, b"altered executable", signature).is_err());
    assert!(ratatoskr_core::updates::verify(key, payload, "invalid signature").is_err());
    let settings = ratatoskr_core::updates::UpdateSettings {
        endpoint: "http://example.com/latest.json".into(),
        public_key: key.into(),
        automatic: false,
    };
    assert!(settings.validate().is_err());
}

#[test]
fn source_configuration_editing_validates_templates_and_detects_external_edits() {
    let f = Fixture::new();
    let document = ratatoskr_core::read_source_file(&f.settings, "harnesses.toml").unwrap();
    assert!(
        ratatoskr_core::write_source_file(&f.settings, "harnesses.toml", "broken=", &document.hash)
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(f.settings.source.join("harnesses.toml")).unwrap(),
        document.text
    );
    ratatoskr_core::write_source_file(
        &f.settings,
        "harnesses.toml",
        &format!("{}\n# edited\n", document.text),
        &document.hash,
    )
    .unwrap();
    assert!(
        ratatoskr_core::write_source_file(
            &f.settings,
            "harnesses.toml",
            &document.text,
            &document.hash
        )
        .is_err()
    );
    assert!(ratatoskr_core::read_source_file(&f.settings, "../outside.toml").is_err());
}

use super::*;

#[test]
fn sync_alerts_once_for_project_skill_work_left_pending_past_the_grace_period() {
    let mut f = Fixture::new();
    f.settings.updates.automatic = false;
    f.settings.alert_command = vec![
        "git".into(),
        "-C".into(),
        f.settings.source.display().to_string(),
        "config".into(),
        "--add".into(),
        "ratatoskr.alert".into(),
    ];
    for (name, checkout) in [
        ("content", &f.settings.source),
        ("project", &f.root.join("project")),
    ] {
        let remote = f.root.join(format!("{name}.git"));
        fs::create_dir_all(&remote).unwrap();
        git(&remote, &["init", "--bare", "-b", "main"]);
        fs::create_dir_all(checkout).unwrap();
        git(checkout, &["init", "-b", "main"]);
        git(checkout, &["config", "user.name", "Ratatoskr tests"]);
        git(checkout, &["config", "user.email", "tests@localhost"]);
        git(checkout, &["config", "commit.gpgsign", "false"]);
        if name == "project" {
            fs::create_dir_all(checkout.join("agent-skills/skills/notes")).unwrap();
            fs::write(checkout.join(".ratatoskr.toml"), "source='agent-skills'\n").unwrap();
            fs::write(checkout.join("agent-skills/skills/notes/skill.toml"), "").unwrap();
            fs::write(
                checkout.join("agent-skills/skills/notes/SKILL.md"),
                "---\nname: notes\ndescription: Take notes\n---\nInitial\n",
            )
            .unwrap();
        }
        git(checkout, &["add", "."]);
        git(checkout, &["commit", "-m", "Initial"]);
        git(
            checkout,
            &["remote", "add", "origin", &format!("../{name}.git")],
        );
        git(checkout, &["push", "-u", "origin", "main"]);
    }
    let project = f.root.join("project");
    let config = f.root.join("config.toml");
    save(&config, &f.settings).unwrap();
    add_project(&config, &project, "agent-skills").unwrap();
    let settings = load(&config).unwrap();
    let alerts = || {
        Command::new("git")
            .arg("-C")
            .arg(&settings.source)
            .args(["config", "--get-all", "ratatoskr.alert"])
            .output()
            .map(|output| String::from_utf8(output.stdout).unwrap())
            .unwrap()
    };

    // Work in progress syncs without an alert.
    let skill = project.join("agent-skills/skills/notes");
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: notes\ndescription: Take notes\n---\nDraft\n",
    )
    .unwrap();
    sync(&settings).unwrap();
    assert!(f.output().exists());
    assert_eq!(alerts(), "");

    // Once the work is an hour old, one alert names it without file counts.
    let ledger_path = settings.state_dir.join("test/project-work.json");
    let mut ledger: serde_json::Value =
        serde_json::from_slice(&fs::read(&ledger_path).unwrap()).unwrap();
    for since in ledger["first_seen"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .flat_map(|items| items.as_object_mut().unwrap().values_mut())
    {
        *since = 0.into();
    }
    fs::write(&ledger_path, serde_json::to_vec(&ledger).unwrap()).unwrap();
    let error = format!("{:#}", sync(&settings).unwrap_err());
    assert!(
        error.contains(&format!(
            "project skill work pending for over an hour: {} (skill notes)",
            project.display()
        )),
        "{error}"
    );
    assert_eq!(alerts().lines().count(), 1);
    fs::create_dir(skill.join("scripts")).unwrap();
    fs::write(skill.join("scripts/run.sh"), "echo notes").unwrap();
    assert!(sync(&settings).is_err());
    assert_eq!(alerts().lines().count(), 1);

    git(&project, &["add", "."]);
    git(&project, &["commit", "-m", "Notes"]);
    git(&project, &["push"]);
    sync(&settings).unwrap();
    assert_eq!(alerts().lines().count(), 2);
    assert!(alerts().contains("operation recovered"));
}

#[test]
fn sync_refuses_dirty_content_without_modifying_it() {
    let f = Fixture::new();
    git(&f.settings.source, &["init", "-b", "main"]);
    assert!(sync(&f.settings).is_err());
    assert!(!f.settings.state_dir.join("test/staging").exists());
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
    #[cfg(windows)]
    {
        let error = ratatoskr_core::schedule_plan(&f.settings, &config, &helper, 60)
            .err()
            .unwrap();
        assert!(format!("{error:#}").contains("tray sync"));
        assert!(!f.settings.state_dir.exists());
    }
    #[cfg(not(windows))]
    {
        let schedule = ratatoskr_core::schedule_plan(&f.settings, &config, &helper, 60).unwrap();
        assert!(schedule.definition.contains("&amp;"));
        assert!(!schedule.definition.contains("&amp;amp;"));
        assert!(!f.settings.state_dir.exists());
        assert!(ratatoskr_core::schedule_plan(&f.settings, &config, &helper, 1).is_err());
    }
}

#[test]
fn content_setup_creates_real_host_profile_and_stays_in_staging() {
    let f = Fixture::new();
    let root = f.root.join("new-content");
    let config = f.root.join("new-config.toml");
    let settings = ratatoskr_core::create_content(&config, &root, "draft", None).unwrap();
    assert_eq!(plan(&settings).unwrap().mode, "staging");
    assert_eq!(plan(&settings).unwrap().targets, 2);
    assert!(root.join(".git").is_dir());
    assert!(ratatoskr_core::create_content(&config, &root, "draft", None).is_err());
}

#[test]
fn signed_updates_accept_tauri_signatures_and_reject_tampering() {
    let key = include_str!("../fixtures/update.pub");
    let payload = include_bytes!("../fixtures/update.txt");
    let signature = include_str!("../fixtures/update.txt.sig");
    ratatoskr_core::updates::verify(key, payload, signature).unwrap();
    assert!(ratatoskr_core::updates::verify(key, b"altered executable", signature).is_err());
    assert!(ratatoskr_core::updates::verify(key, payload, "invalid signature").is_err());
    let settings = ratatoskr_core::updates::UpdateSettings {
        endpoint: Some("http://example.com/latest.json".into()),
        public_key: Some(key.into()),
        automatic: false,
        ..Default::default()
    };
    assert!(settings.validate().is_err());
}

#[test]
fn sync_refuses_wrong_branch_upstream_and_detached_head_before_fetch_or_apply() {
    let mut f = Fixture::new();
    f.settings.updates.automatic = false;
    f.settings.alert_command = vec![
        "git".into(),
        "-C".into(),
        f.settings.source.display().to_string(),
        "config".into(),
        "ratatoskr.last-alert".into(),
    ];
    let remote = f.root.join("remote.git");
    fs::create_dir(&remote).unwrap();
    git(&remote, &["init", "--bare", "-b", "main"]);
    let source = &f.settings.source;
    git(source, &["init", "-b", "main"]);
    git(source, &["config", "user.name", "Ratatoskr tests"]);
    git(source, &["config", "user.email", "tests@localhost"]);
    git(source, &["config", "commit.gpgsign", "false"]);
    git(source, &["add", "."]);
    git(source, &["commit", "-m", "Content"]);
    // A relative local remote also works with Windows verbatim canonical paths.
    git(source, &["remote", "add", "origin", "../remote.git"]);
    git(source, &["push", "-u", "origin", "main"]);
    git(source, &["switch", "-c", "experiment"]);
    git(source, &["push", "-u", "origin", "experiment"]);
    let assert_refused = || {
        assert!(sync(&f.settings).is_err());
        assert!(!source.join(".git/FETCH_HEAD").exists());
        assert!(!f.output().exists());
        let alert =
            fs::read_to_string(f.settings.state_dir.join("test/notification.json")).unwrap();
        assert!(alert.contains("error"));
    };
    assert_refused();
    git(source, &["switch", "main"]);
    git(source, &["branch", "--set-upstream-to=origin/experiment"]);
    assert_refused();
    git(source, &["branch", "--unset-upstream"]);
    assert_refused();
    git(source, &["checkout", "--detach"]);
    assert_refused();
    git(source, &["switch", "main"]);
    git(source, &["branch", "--set-upstream-to=origin/main"]);
    sync(&f.settings).unwrap();
    assert!(f.output().exists());
    f.settings.deploy_branch = "experiment".into();
    git(&f.settings.source, &["switch", "experiment"]);
    sync(&f.settings).unwrap();
}

#[test]
fn helper_detection_prefers_bundled_sibling_then_standalone_and_reports_absence() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let bundled = root.join("bundle");
    let standalone = root.join("bin");
    fs::create_dir_all(&bundled).unwrap();
    fs::create_dir_all(&standalone).unwrap();
    let name = if cfg!(windows) {
        "ratatoskr.exe"
    } else {
        "ratatoskr"
    };
    let executable = bundled.join("ratatoskr-desktop");
    let search = std::env::join_paths([&standalone]).unwrap();
    assert!(ratatoskr_core::detect_helper(&executable, Some(&search)).is_err());
    for directory in [&standalone, &bundled] {
        let path = directory.join(name);
        fs::write(&path, b"helper").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(
            ratatoskr_core::detect_helper(&executable, Some(&search)).unwrap(),
            path
        );
    }
}

#[test]
fn background_status_records_failure_and_ledger_without_alert_command() {
    let f = Fixture::new();
    assert!(sync(&f.settings).is_err()); // Not a clean deployment Git checkout.
    let status = ratatoskr_core::job_status(&f.settings).unwrap();
    assert!(status.last_run.as_ref().unwrap().finished_at.is_some());
    assert!(status.last_run.unwrap().error.is_some());
    assert!(status.current_error.is_some());
    assert!(
        ratatoskr_core::job_log(&f.settings)
            .unwrap()
            .contains("Sync failed")
    );
}

#[test]
fn tray_runs_record_success_conflicts_and_configuration_errors_in_existing_ledgers_and_logs() {
    let f = Fixture::new();
    let source = &f.settings.source;
    let remote = f.root.join("remote.git");
    fs::create_dir(&remote).unwrap();
    git(&remote, &["init", "--bare", "-b", "main"]);
    git(source, &["init", "-b", "main"]);
    git(source, &["config", "user.name", "Ratatoskr tests"]);
    git(source, &["config", "user.email", "tests@localhost"]);
    git(source, &["config", "commit.gpgsign", "false"]);
    git(source, &["add", "."]);
    git(source, &["commit", "-m", "Content"]);
    git(source, &["remote", "add", "origin", "../remote.git"]);
    git(source, &["push", "-u", "origin", "main"]);
    let config = f.root.join("config.toml");
    save(&config, &f.settings).unwrap();
    let report = ratatoskr_core::recorded_sync(&config).unwrap().unwrap();
    assert_eq!(report.skills, 1);
    let status = ratatoskr_core::job_status(&f.settings).unwrap();
    assert!(status.last_run.as_ref().unwrap().finished_at.is_some());
    assert!(status.last_run.as_ref().unwrap().error.is_none());
    assert!(
        ratatoskr_core::job_log(&f.settings)
            .unwrap()
            .contains("sync.stdout.log")
    );
    fs::write(f.output().join("SKILL.md"), "Hand edit").unwrap();
    assert!(ratatoskr_core::recorded_sync(&config).is_err());
    let status = ratatoskr_core::job_status(&f.settings).unwrap();
    assert!(
        status
            .last_run
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("conflicts")
    );
    assert!(
        ratatoskr_core::job_log(&f.settings)
            .unwrap()
            .contains("sync.stderr.log")
    );
    assert_eq!(
        fs::read_to_string(f.output().join("SKILL.md")).unwrap(),
        "Hand edit"
    );
    fs::write(&config, "malformed = [").unwrap();
    assert!(ratatoskr_core::recorded_sync(&config).is_err());
    let bootstrap = config
        .with_file_name("config.toml.diagnostics")
        .join("startup");
    let run: ratatoskr_core::JobRun =
        serde_json::from_slice(&fs::read(bootstrap.join("last-sync.json")).unwrap()).unwrap();
    assert!(run.error.is_some() && run.finished_at.is_some());
    assert!(
        !fs::read_to_string(bootstrap.join("sync.stderr.log"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn an_upstream_with_the_right_short_name_in_another_namespace_cannot_deploy() {
    let f = Fixture::new();
    let source = &f.settings.source;
    git(source, &["init", "-b", "main"]);
    git(source, &["config", "user.email", "test@example.com"]);
    git(source, &["config", "user.name", "Test"]);
    git(source, &["add", "-A"]);
    git(source, &["commit", "-m", "Initial"]);
    let remote = f.root.join("remote.git");
    git(&f.root, &["init", "--bare", "-b", "main", "remote.git"]);
    git(source, &["remote", "add", "origin", &git_path(&remote)]);
    git(source, &["push", "origin", "main"]);
    git(
        source,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/heads/origin/*",
        ],
    );
    git(source, &["fetch", "origin"]);
    git(source, &["config", "branch.main.remote", "origin"]);
    git(source, &["config", "branch.main.merge", "refs/heads/main"]);
    let mut settings = f.settings.clone();
    settings.deploy_branch = "main".into();
    let error = ratatoskr_core::content_status(&settings).error.unwrap();
    assert!(
        error.contains("expected refs/remotes/origin/main"),
        "{error}"
    );
}

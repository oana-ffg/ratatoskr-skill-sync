use super::*;

fn clock(value: &str) -> chrono::DateTime<chrono::FixedOffset> {
    chrono::DateTime::parse_from_rfc3339(value).unwrap()
}
fn nightly_due(
    settings: &UpdateSettings,
    now: chrono::DateTime<chrono::FixedOffset>,
    last_check: Option<chrono::DateTime<chrono::FixedOffset>>,
) -> bool {
    DesktopUpdateState {
        last_check,
        settings: Some(settings.clone()),
        pending: false,
        ..Default::default()
    }
    .due(settings, now, false)
    .unwrap()
}
#[test]
fn official_defaults_and_explicit_overrides() {
    let defaults: UpdateSettings = toml::from_str("").unwrap();
    defaults.validate().unwrap();
    verify(
        OFFICIAL_PUBLIC_KEY,
        include_bytes!("../../tests/fixtures/update.txt"),
        include_str!("../../tests/fixtures/official-update.txt.sig"),
    )
    .unwrap();
    assert_eq!(defaults.endpoint(), OFFICIAL_ENDPOINT);
    assert_eq!(defaults.public_key(), OFFICIAL_PUBLIC_KEY.trim());
    assert!(!defaults.override_active());
    assert!(defaults.automatic);
    assert_eq!(defaults.time, "00:00");
    let settings: crate::Settings =
        toml::from_str("source='/content'\nmachine='test'\nstate_dir='/state'").unwrap();
    assert_eq!(settings.deploy_branch, "main");
    assert_eq!(settings.updates, defaults);
    let overridden: UpdateSettings =
        toml::from_str("endpoint='https://example.com/fork.json'").unwrap();
    assert_eq!(overridden.endpoint(), "https://example.com/fork.json");
    assert_eq!(overridden.public_key(), defaults.public_key());
    assert!(overridden.override_active());
    let key_only = UpdateSettings {
        public_key: Some(
            include_str!("../../tests/fixtures/update.pub")
                .trim()
                .into(),
        ),
        ..Default::default()
    };
    key_only.validate().unwrap();
    assert_eq!(key_only.endpoint(), OFFICIAL_ENDPOINT);
    assert!(key_only.override_active());
    assert_ne!(key_only.public_key(), defaults.public_key());
    let config: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../desktop/src-tauri/tauri.conf.json"
    ))
    .unwrap();
    assert_eq!(
        config["plugins"]["updater"]["pubkey"],
        defaults.public_key()
    );
    assert_eq!(
        config["plugins"]["updater"]["endpoints"][0],
        OFFICIAL_ENDPOINT
    );
    assert!(
        UpdateSettings {
            endpoint: Some(String::new()),
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        UpdateSettings {
            public_key: Some(String::new()),
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}
#[test]
fn local_clock_controls_daily_checks_and_bounded_desktop_window() {
    let settings = UpdateSettings {
        time: "03:15".into(),
        ..Default::default()
    };
    assert!(nightly_due(
        &settings,
        clock("2026-10-16T03:14:59+02:00"),
        None
    ));
    let now = clock("2026-10-16T03:15:00+02:00");
    assert!(!desktop_window(&settings, clock("2026-10-16T03:14:59+02:00")).unwrap());
    assert!(desktop_window(&settings, now).unwrap());
    assert!(!desktop_window(&settings, clock("2026-10-16T04:15:00+02:00")).unwrap());
    assert!(nightly_due(&settings, now, None));
    assert!(!helper_check_due(&settings, now, Some(now)).unwrap());
    assert!(helper_check_due(&settings, clock("2026-10-17T03:15:00+02:00"), Some(now)).unwrap());
    assert!(nightly_due(
        &settings,
        clock("2026-10-16T04:15:00+02:00"),
        None
    ));
    assert!(helper_check_due(&settings, clock("2026-10-16T04:15:00+02:00"), None).unwrap());
    assert!(
        !helper_check_due(
            &UpdateSettings {
                automatic: false,
                ..settings
            },
            now,
            None
        )
        .unwrap()
    );
    assert!(
        UpdateSettings {
            time: "25:00".into(),
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}
#[test]
fn desktop_window_spans_midnight_and_handles_local_offset_changes() {
    let settings = UpdateSettings {
        time: "23:30".into(),
        ..Default::default()
    };
    let checked = clock("2026-10-16T23:30:00+02:00");
    let after_midnight = clock("2026-10-17T00:15:00+02:00");
    assert!(desktop_window(&settings, after_midnight).unwrap());
    assert!(!nightly_due(&settings, after_midnight, Some(checked)));
    assert!(!desktop_window(&settings, clock("2026-10-17T00:30:00+02:00")).unwrap());
    let repeated_hour = UpdateSettings {
        time: "02:15".into(),
        ..Default::default()
    };
    assert!(!nightly_due(
        &repeated_hour,
        clock("2026-10-25T02:30:00+01:00"),
        Some(clock("2026-10-25T02:15:00+02:00"))
    ));
    let midnight = UpdateSettings::default();
    assert!(
        helper_check_due(
            &midnight,
            clock("2026-10-26T00:00:00+01:00"),
            Some(clock("2026-10-25T00:00:00+02:00"))
        )
        .unwrap()
    );
}
#[test]
fn desktop_launch_catches_up_missed_windows_and_persists_decisions() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = crate::Settings {
        state_dir: temp.path().canonicalize().unwrap(),
        machine: "test".into(),
        ..Default::default()
    };
    let now = clock("2026-10-16T09:00:00+02:00");
    let path = settings.state_dir.join("test/desktop-update.json");
    let mut state = DesktopUpdateState::load(&path).unwrap();
    assert!(state.due(&settings.updates, now, true).unwrap());
    assert!(state.due(&settings.updates, now, false).unwrap());
    state.record_attempt(&path, &settings.updates, now).unwrap();
    let mut state = DesktopUpdateState::load(&path).unwrap();
    assert!(!state.due(&settings.updates, now, true).unwrap());
    assert!(
        state
            .due(&settings.updates, clock("2026-10-17T00:00:00+02:00"), false)
            .unwrap()
    );
    assert!(
        state
            .due(&settings.updates, clock("2026-10-17T09:00:00+02:00"), true)
            .unwrap()
    );
    state.record_available(&path, true).unwrap();
    let mut state = DesktopUpdateState::load(&path).unwrap();
    assert!(
        state.due(&settings.updates, now, true).unwrap(),
        "deferred installation resumes on launch"
    );
    assert!(!state.due(&settings.updates, now, false).unwrap());
    state.record_available(&path, false).unwrap();
    assert!(
        !DesktopUpdateState::load(&path)
            .unwrap()
            .due(&settings.updates, now, true)
            .unwrap()
    );
    settings.updates.time = "23:30".into();
    assert!(
        state.due(&settings.updates, now, true).unwrap(),
        "changed settings get a new check"
    );
    state
        .record_attempt(&path, &settings.updates, clock("2026-10-15T23:45:00+02:00"))
        .unwrap();
    assert!(
        !state
            .due(&settings.updates, clock("2026-10-16T00:15:00+02:00"), true)
            .unwrap()
    );
    assert!(
        state
            .due(&settings.updates, clock("2026-10-17T09:00:00+02:00"), true)
            .unwrap()
    );
    settings.updates.automatic = false;
    assert!(!state.due(&settings.updates, now, true).unwrap());
    settings.updates.automatic = true;
    settings.updates.time = "invalid".into();
    assert!(state.due(&settings.updates, now, true).is_err());
    std::fs::write(
        settings.state_dir.join("test/desktop-update.json"),
        "corrupt",
    )
    .unwrap();
    assert!(DesktopUpdateState::load(&path).is_err());
}

#[test]
fn resident_checks_catch_up_after_sleep_and_days_of_uptime() {
    let settings = UpdateSettings::default();
    let temp = tempfile::tempdir().unwrap();
    let path = temp
        .path()
        .canonicalize()
        .unwrap()
        .join("desktop-update.json");
    let mut state = DesktopUpdateState::default();
    let checked = clock("2026-10-18T00:01:00+02:00");
    state.record_attempt(&path, &settings, checked).unwrap();
    state.record_success(&path, checked, false).unwrap();
    for resume in ["2026-10-19T13:38:00+02:00", "2026-10-25T13:38:00+01:00"] {
        let mut resumed = DesktopUpdateState::load(&path).unwrap();
        let now = clock(resume);
        assert!(!desktop_window(&settings, now).unwrap());
        assert!(resumed.due(&settings, now, false).unwrap());
        resumed.record_attempt(&path, &settings, now).unwrap();
        assert!(resumed.installation_allowed(&settings, now, false).unwrap());
        assert!(!resumed.due(&settings, now, false).unwrap());
        let reopened = DesktopUpdateState::load(&path).unwrap();
        assert!(
            reopened
                .installation_allowed(&settings, now + chrono::Duration::minutes(59), false)
                .unwrap()
        );
        assert!(
            !reopened
                .installation_allowed(&settings, now + chrono::Duration::hours(1), false)
                .unwrap()
        );
    }
    let normal = clock("2026-10-26T00:01:00+01:00");
    state.record_attempt(&path, &settings, normal).unwrap();
    assert!(
        state
            .installation_allowed(&settings, clock("2026-10-26T00:59:59+01:00"), false)
            .unwrap()
    );
    assert!(
        !state
            .installation_allowed(&settings, clock("2026-10-26T01:00:00+01:00"), false)
            .unwrap()
    );
}

#[test]
fn overdue_tracks_success_not_attempts_and_migrates_legacy_state_with_grace() {
    let settings = UpdateSettings::default();
    let temp = tempfile::tempdir().unwrap();
    let path = temp
        .path()
        .canonicalize()
        .unwrap()
        .join("desktop-update.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "last_check": "2026-10-18T00:01:00+02:00", "settings": settings, "pending": false
        }))
        .unwrap(),
    )
    .unwrap();
    let mut state = DesktopUpdateState::load(&path).unwrap();
    let started = clock("2026-10-19T13:38:00+02:00");
    state.observe(&path, &settings, started, false).unwrap();
    assert!(!state.overdue(&settings, started).unwrap());
    for value in ["2026-10-20T13:38:00+02:00", "2026-10-21T00:59:59+02:00"] {
        assert!(!state.overdue(&settings, clock(value)).unwrap());
    }
    let overdue = clock("2026-10-21T01:00:00+02:00");
    state.record_attempt(&path, &settings, overdue).unwrap();
    assert!(
        DesktopUpdateState::load(&path)
            .unwrap()
            .overdue(&settings, overdue)
            .unwrap()
    );
    assert!(
        state
            .overdue(&settings, clock("2026-10-22T00:30:00+02:00"))
            .unwrap()
    );
    state.record_success(&path, overdue, false).unwrap();
    assert!(!state.overdue(&settings, overdue).unwrap());
    state
        .record_success(&path, clock("2026-10-24T00:01:00+02:00"), false)
        .unwrap();
    assert!(
        !state
            .overdue(&settings, clock("2026-10-25T13:38:00+01:00"))
            .unwrap()
    );
    assert!(
        state
            .overdue(&settings, clock("2026-10-26T13:38:00+01:00"))
            .unwrap()
    );
    let reopened = clock("2026-10-28T13:38:00+01:00");
    state.observe(&path, &settings, reopened, true).unwrap();
    assert_eq!(
        state.overdue(&settings, reopened).unwrap(),
        cfg!(target_os = "macos"),
        "macOS must preserve health while closed; other platforms retain resident-session accounting"
    );
    assert!(
        state
            .overdue(&settings, clock("2026-10-30T01:00:00+01:00"))
            .unwrap()
    );
    let disabled = UpdateSettings {
        automatic: false,
        ..settings.clone()
    };
    assert!(
        !state
            .overdue(&disabled, clock("2026-10-25T13:38:00+01:00"))
            .unwrap()
    );
    let changed = UpdateSettings {
        time: "23:30".into(),
        ..settings
    };
    state.observe(&path, &changed, overdue, false).unwrap();
    assert!(state.due(&changed, overdue, false).unwrap());
    assert!(!state.overdue(&changed, overdue).unwrap());
}

#[test]
fn helper_persists_attempts_including_failures_and_retries_next_day() {
    let temp = tempfile::tempdir().unwrap();
    let settings = crate::Settings {
        state_dir: temp.path().canonicalize().unwrap(),
        machine: "test".into(),
        ..Default::default()
    };
    let now = clock("2026-10-16T00:00:00+02:00");
    assert!(helper_update_with(&settings, now, || anyhow::bail!("offline")).is_err());
    helper_update_with(&settings, clock("2026-10-16T23:59:00+02:00"), || {
        panic!("must not check twice")
    })
    .unwrap();
    let mut checked = false;
    helper_update_with(&settings, clock("2026-10-17T00:00:00+02:00"), || {
        checked = true;
        Ok(())
    })
    .unwrap();
    assert!(checked);
    let path = settings.state_dir.join("test/helper-update.json");
    std::fs::write(path, "corrupt").unwrap();
    assert!(
        helper_update_with(&settings, now, || panic!("invalid state must fail closed")).is_err()
    );
}

fn alert_settings(temp: &tempfile::TempDir) -> crate::Settings {
    let root = dunce::canonicalize(temp.path()).unwrap();
    let settings = crate::Settings {
        source: root.join("source"),
        state_dir: root.join("state"),
        machine: "test".into(),
        ..Default::default()
    };
    std::fs::create_dir(&settings.source).unwrap();
    std::fs::create_dir(&settings.state_dir).unwrap();
    settings
}

fn enable_recorder(settings: &mut crate::Settings) -> std::path::PathBuf {
    let path = settings.state_dir.join("alerts.gitconfig");
    settings.alert_command = vec![
        "git".into(),
        "config".into(),
        "--file".into(),
        path.to_string_lossy().into_owned(),
        "--add".into(),
        "alerts.message".into(),
    ];
    path
}
fn alert_count(path: &std::path::Path) -> usize {
    if !path.exists() {
        return 0;
    }
    let output = crate::process::command("git")
        .args(["config", "--file"])
        .arg(path)
        .args(["--get-all", "alerts.message"])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().lines().count()
}

#[test]
fn skipped_helper_check_delivers_previously_unsent_error_and_recovers_once() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = alert_settings(&temp);
    let now = clock("2026-10-20T00:01:00+02:00");
    assert!(helper_update_with(&settings, now, || anyhow::bail!("offline")).is_err());
    let ledger = settings
        .state_dir
        .join("test/helper-update-notification.json");
    let unsent: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&ledger).unwrap()).unwrap();
    assert_eq!(unsent["sent_at"], 0);
    let alerts = enable_recorder(&mut settings);
    for _ in 0..2 {
        helper_update_with(&settings, now, || panic!("daily attempt already consumed")).unwrap();
    }
    assert_eq!(alert_count(&alerts), 1);
    helper_update_with(&settings, clock("2026-10-21T00:01:00+02:00"), || Ok(())).unwrap();
    helper_update_with(
        &settings,
        clock("2026-10-21T00:02:00+02:00"),
        || unreachable!(),
    )
    .unwrap();
    assert_eq!(alert_count(&alerts), 2);
    assert!(
        crate::job_status(&settings)
            .unwrap()
            .helper_update_error
            .is_none()
    );
}

#[test]
fn bundled_helpers_clear_obsolete_errors_without_attempting_self_update() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = alert_settings(&temp);
    let alerts = enable_recorder(&mut settings);
    report_outcome(&settings, Some(BUNDLED_HELPER_ERROR), UpdateKind::Helper).unwrap();
    assert_eq!(alert_count(&alerts), 1);
    for _ in 0..2 {
        helper_update_for(&settings, false, clock("2026-10-20T00:01:00+02:00"), || {
            panic!("bundled helpers must not check or install helper updates")
        })
        .unwrap();
    }
    assert!(
        crate::job_status(&settings)
            .unwrap()
            .helper_update_error
            .is_none()
    );
    assert_eq!(alert_count(&alerts), 2);
    report_outcome(
        &settings,
        Some("Helper update: unknown failure"),
        UpdateKind::Helper,
    )
    .unwrap();
    helper_update_for(
        &settings,
        false,
        clock("2026-10-20T00:01:00+02:00"),
        || unreachable!(),
    )
    .unwrap();
    assert_eq!(
        crate::job_status(&settings)
            .unwrap()
            .helper_update_error
            .as_deref(),
        Some("Helper update: unknown failure")
    );
    // An obsolete error never delivered does not generate a false recovery.
    let mut quiet = settings.clone();
    quiet.machine = "quiet".into();
    quiet.alert_command.clear();
    report_outcome(&quiet, Some(BUNDLED_HELPER_ERROR), UpdateKind::Helper).unwrap();
    quiet.alert_command = settings.alert_command.clone();
    helper_update_for(
        &quiet,
        false,
        clock("2026-10-20T00:01:00+02:00"),
        || unreachable!(),
    )
    .unwrap();
    assert_eq!(alert_count(&alerts), 3);
}

#[test]
fn helper_watchdog_observes_stopped_desktop_deduplicates_and_recovers() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = alert_settings(&temp);
    let alerts = enable_recorder(&mut settings);
    let path = settings.state_dir.join("app/desktop-update.json");
    let first = clock("2026-10-18T00:01:00+02:00");
    let mut state = DesktopUpdateState::default();
    state
        .observe(&path, &settings.updates, first, false)
        .unwrap();
    state.record_success(&path, first, false).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    desktop_watchdog_with(&settings, &path, clock("2026-10-20T00:59:59+02:00")).unwrap();
    assert_eq!(alert_count(&alerts), 0);
    let overdue = clock("2026-10-20T01:00:00+02:00");
    for _ in 0..2 {
        desktop_watchdog_with(&settings, &path, overdue).unwrap();
    }
    assert_eq!(alert_count(&alerts), 1);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "helper must not modify desktop-owned state"
    );
    assert_eq!(
        crate::job_status(&settings)
            .unwrap()
            .desktop_update_error
            .as_deref(),
        Some(DESKTOP_OVERDUE)
    );
    // Existing desktop reporter and helper share delivery history.
    report_outcome(&settings, Some(DESKTOP_OVERDUE), UpdateKind::Desktop).unwrap();
    assert_eq!(alert_count(&alerts), 1);
    // Simulate a delivered alert older than 24 hours.
    let ledger = settings
        .state_dir
        .join("test/desktop-update-notification.json");
    let mut notification: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&ledger).unwrap()).unwrap();
    notification["sent_at"] = 1.into();
    std::fs::write(&ledger, serde_json::to_vec(&notification).unwrap()).unwrap();
    desktop_watchdog_with(&settings, &path, overdue).unwrap();
    assert_eq!(alert_count(&alerts), 2);
    state.record_success(&path, overdue, false).unwrap();
    for _ in 0..2 {
        desktop_watchdog_with(&settings, &path, overdue).unwrap();
    }
    assert_eq!(alert_count(&alerts), 3);
    assert!(
        crate::job_status(&settings)
            .unwrap()
            .desktop_update_error
            .is_none()
    );
}

#[test]
fn helper_watchdog_missing_and_legacy_state_have_bounded_grace() {
    for legacy in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let settings = alert_settings(&temp);
        let path = settings.state_dir.join("desktop-update.json");
        if legacy {
            std::fs::write(&path, serde_json::to_vec(&serde_json::json!({
                "last_check": "2026-10-01T00:01:00+02:00", "settings": settings.updates, "pending": false
            })).unwrap()).unwrap();
        }
        let first = clock("2026-10-20T12:00:00+02:00");
        desktop_watchdog_with(&settings, &path, first).unwrap();
        desktop_watchdog_with(&settings, &path, clock("2026-10-22T00:59:59+02:00")).unwrap();
        assert!(
            crate::job_status(&settings)
                .unwrap()
                .desktop_update_error
                .is_none()
        );
        desktop_watchdog_with(&settings, &path, clock("2026-10-22T01:00:00+02:00")).unwrap();
        assert_eq!(
            crate::job_status(&settings)
                .unwrap()
                .desktop_update_error
                .as_deref(),
            Some(DESKTOP_OVERDUE)
        );
    }
}

#[test]
fn helper_watchdog_reports_unverifiable_state_retries_specific_errors_and_respects_opt_out() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = alert_settings(&temp);
    let path = settings.state_dir.join("desktop-update.json");
    let now = clock("2026-10-20T12:00:00+02:00");
    std::fs::write(&path, "broken").unwrap();
    assert!(desktop_watchdog_with(&settings, &path, now).is_err());
    assert!(
        crate::job_status(&settings)
            .unwrap()
            .desktop_update_error
            .unwrap()
            .starts_with(MONITORING_FAILED)
    );
    std::fs::remove_file(&path).unwrap();
    desktop_watchdog_with(&settings, &path, now).unwrap();
    report_outcome(
        &settings,
        Some("Desktop update: bad signature"),
        UpdateKind::Desktop,
    )
    .unwrap();
    let alerts = enable_recorder(&mut settings);
    for _ in 0..2 {
        desktop_watchdog_with(&settings, &path, clock("2026-10-23T12:00:00+02:00")).unwrap();
    }
    assert_eq!(alert_count(&alerts), 1);
    assert_eq!(
        crate::job_status(&settings)
            .unwrap()
            .desktop_update_error
            .as_deref(),
        Some("Desktop update: bad signature")
    );
    report_outcome(&settings, None, UpdateKind::Desktop).unwrap();
    settings.updates.automatic = false;
    std::fs::write(&path, "broken but automatic updates are opted out").unwrap();
    desktop_watchdog_with(&settings, &path, clock("2026-11-01T12:00:00+01:00")).unwrap();
    assert!(
        crate::job_status(&settings)
            .unwrap()
            .desktop_update_error
            .is_none()
    );
}

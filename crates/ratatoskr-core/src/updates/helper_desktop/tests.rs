use super::*;
use crate::updates::tests::{alert_count, alert_settings, clock, enable_recorder};
fn exe() -> PathBuf {
    PathBuf::from("/fixture/Ratatoskr.app/Contents/MacOS/ratatoskr")
}
fn release() -> Result<Option<Release>> {
    Ok(Some(Release {
        version: "999.0.0".into(),
        notes: String::new(),
        platforms: BTreeMap::new(),
    }))
}
fn run(
    settings: &crate::Settings,
    data: &Path,
    now: Time,
    check: impl FnOnce() -> Result<Option<Release>>,
) -> Result<()> {
    tick(
        settings,
        data,
        &exe(),
        "0.2.21",
        now,
        check,
        || Ok(Some(exe().with_file_name("ratatoskr-desktop"))),
        |_| panic!("app is running"),
    )
}
fn ledger(settings: &crate::Settings) -> PathBuf {
    settings.state_dir.join(&settings.machine).join(LEDGER)
}
fn error(settings: &crate::Settings) -> Option<String> {
    crate::job_status(settings).unwrap().desktop_update_error
}
#[test]
fn current_app_with_ui_closed_for_days_sends_no_alert() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = alert_settings(&temp);
    let alerts = enable_recorder(&mut settings);
    for day in [18, 19, 20, 25, 30] {
        run(
            &settings,
            &dunce::canonicalize(temp.path()).unwrap(),
            clock(&format!("2026-10-{day}T13:00:00+02:00")),
            || Ok(None),
        )
        .unwrap();
        assert!(error(&settings).is_none());
    }
    assert_eq!(alert_count(&alerts), 0);
}
#[test]
fn due_catches_up_after_sleep_success_skips_until_next_slot_and_noop_never_writes() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = alert_settings(&temp);
    settings.updates.time = "03:15".into();
    let now = clock("2026-10-20T14:00:00+02:00");
    run(
        &settings,
        &dunce::canonicalize(temp.path()).unwrap(),
        now,
        || Ok(None),
    )
    .unwrap();
    let path = ledger(&settings);
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&path).unwrap().ino()
    };
    for time in ["2026-10-20T14:01:00+02:00", "2026-10-21T03:14:59+02:00"] {
        run(
            &settings,
            &dunce::canonicalize(temp.path()).unwrap(),
            clock(time),
            || panic!("not due"),
        )
        .unwrap();
    }
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
    }
    run(
        &settings,
        &dunce::canonicalize(temp.path()).unwrap(),
        clock("2026-10-21T03:15:00+02:00"),
        || Ok(None),
    )
    .unwrap();
    run(
        &settings,
        &dunce::canonicalize(temp.path()).unwrap(),
        clock("2026-10-25T13:00:00+01:00"),
        || Ok(None),
    )
    .unwrap();
}
#[test]
fn failure_retries_hourly_transient_recovery_is_silent_two_failed_windows_include_cause() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = alert_settings(&temp);
    let alerts = enable_recorder(&mut settings);
    let now = clock("2026-10-20T12:00:00+02:00");
    assert!(
        run(
            &settings,
            &dunce::canonicalize(temp.path()).unwrap(),
            now,
            || anyhow::bail!("TLS unavailable")
        )
        .is_err()
    );
    run(
        &settings,
        &dunce::canonicalize(temp.path()).unwrap(),
        now + chrono::Duration::minutes(59),
        || panic!("hourly pacing"),
    )
    .unwrap();
    run(
        &settings,
        &dunce::canonicalize(temp.path()).unwrap(),
        now + chrono::Duration::hours(1),
        || Ok(None),
    )
    .unwrap();
    assert_eq!(alert_count(&alerts), 0);
    for time in [
        "2026-10-21T00:01:00+02:00",
        "2026-10-22T00:01:00+02:00",
        "2026-10-22T01:01:00+02:00",
    ] {
        assert!(
            run(
                &settings,
                &dunce::canonicalize(temp.path()).unwrap(),
                clock(time),
                || anyhow::bail!("TLS unavailable")
            )
            .is_err()
        );
    }
    assert!(error(&settings).unwrap().contains("TLS unavailable"));
    assert_eq!(alert_count(&alerts), 1);
    run(
        &settings,
        &dunce::canonicalize(temp.path()).unwrap(),
        clock("2026-10-22T02:01:00+02:00"),
        || Ok(None),
    )
    .unwrap();
    run(
        &settings,
        &dunce::canonicalize(temp.path()).unwrap(),
        clock("2026-10-22T02:02:00+02:00"),
        || panic!("current"),
    )
    .unwrap();
    assert_eq!(alert_count(&alerts), 2);
}
#[test]
fn legacy_skirnir_baseline_checks_first_and_recovers_without_fresh_overdue() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = alert_settings(&temp);
    let alerts = enable_recorder(&mut settings);
    crate::config::atomic_write(&ledger(&settings), br#"{"settings":{},"monitoring_since":"2026-10-01T00:00:00+02:00","last_success":null,"pending":false}"#).unwrap();
    report_outcome(&settings, Some(DESKTOP_OVERDUE), UpdateKind::Desktop).unwrap();
    let path = settings
        .state_dir
        .join("test/desktop-update-notification.json");
    let mut old: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    old["sent_at"] = 1.into();
    std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    run(
        &settings,
        &dunce::canonicalize(temp.path()).unwrap(),
        clock("2026-11-08T12:00:00+01:00"),
        || Ok(None),
    )
    .unwrap();
    assert_eq!(alert_count(&alerts), 2); // original alert and one recovery
    assert!(error(&settings).is_none());
}
#[test]
fn automatic_off_and_standalone_ignore_desktop_state_and_clear_stale_monitoring() {
    for standalone in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = alert_settings(&temp);
        settings.updates.automatic = standalone;
        std::fs::write(
            dunce::canonicalize(temp.path())
                .unwrap()
                .join("desktop-update.json"),
            "corrupt old desktop state",
        )
        .unwrap();
        report_outcome(&settings, Some(DESKTOP_OVERDUE), UpdateKind::Desktop).unwrap();
        let executable = if standalone {
            PathBuf::from("/bin/ratatoskr")
        } else {
            exe()
        };
        tick(
            &settings,
            &dunce::canonicalize(temp.path()).unwrap(),
            &executable,
            "0.2.21",
            clock("2026-11-08T12:00:00+01:00"),
            || panic!("disabled or standalone"),
            || panic!("no process probes"),
            |_| panic!("no spawn"),
        )
        .unwrap();
        assert!(!ledger(&settings).exists());
        assert!(error(&settings).is_none());
    }
}
#[test]
fn corrupt_ledger_alerts_and_current_check_clears_monitoring_failure() {
    let temp = tempfile::tempdir().unwrap();
    let settings = alert_settings(&temp);
    crate::config::atomic_write(&ledger(&settings), b"broken").unwrap();
    let now = clock("2026-11-08T12:00:00+01:00");
    assert!(
        run(
            &settings,
            &dunce::canonicalize(temp.path()).unwrap(),
            now,
            || panic!("corrupt")
        )
        .is_err()
    );
    assert!(error(&settings).unwrap().starts_with(MONITORING_FAILED));
    std::fs::remove_file(ledger(&settings)).unwrap();
    run(
        &settings,
        &dunce::canonicalize(temp.path()).unwrap(),
        now,
        || Ok(None),
    )
    .unwrap();
    assert!(error(&settings).is_none());
}
#[test]
fn pending_defers_launches_once_reuses_unanswered_request_and_alerts_after_hour() {
    let temp = tempfile::tempdir().unwrap();
    let settings = alert_settings(&temp);
    let data = dunce::canonicalize(temp.path()).unwrap().join("app-data");
    let now = clock("2026-11-08T12:00:00+01:00");
    run(&settings, &data, now, release).unwrap();
    assert!(desktop_request_status(&data).unwrap().0.is_none());
    let launch = |time| {
        tick(
            &settings,
            &data,
            &exe(),
            "0.2.21",
            time,
            || panic!("already checked"),
            || Ok(None),
            |_| Ok(()),
        )
    };
    launch(now).unwrap();
    let request = desktop_request_status(&data).unwrap().0.unwrap();
    tick(
        &settings,
        &data,
        &exe(),
        "0.2.21",
        now + chrono::Duration::minutes(1),
        || panic!("checked"),
        || panic!("not due"),
        |_| panic!("hourly pacing"),
    )
    .unwrap();
    launch(now + chrono::Duration::hours(1)).unwrap();
    assert_eq!(
        desktop_request_status(&data).unwrap().0.unwrap().id,
        request.id
    );
    assert!(error(&settings).unwrap().contains("999.0.0 did not finish"));
    answer_desktop_request(
        &data,
        &request,
        now,
        DesktopRequestOutcome::Failed {
            error: "bad signature".into(),
        },
    )
    .unwrap();
    launch(now + chrono::Duration::hours(2)).unwrap();
    assert!(error(&settings).unwrap().contains("bad signature"));
    assert_ne!(
        desktop_request_status(&data).unwrap().0.unwrap().id,
        request.id
    );
    tick(
        &settings,
        &data,
        &exe(),
        "999.0.0",
        now + chrono::Duration::hours(2),
        || panic!("same slot"),
        || panic!("updated"),
        |_| panic!("updated"),
    )
    .unwrap();
    assert!(error(&settings).is_none());
}
#[test]
fn launch_failure_is_visible_immediately_and_a_hung_running_installer_is_visible_after_hour() {
    let temp = tempfile::tempdir().unwrap();
    let settings = alert_settings(&temp);
    let now = clock("2026-11-08T12:00:00+01:00");
    tick(
        &settings,
        &dunce::canonicalize(temp.path()).unwrap(),
        &exe(),
        "0.2.21",
        now,
        release,
        || Ok(None),
        |_| anyhow::bail!("cannot execute app"),
    )
    .unwrap();
    assert!(error(&settings).unwrap().contains("cannot execute app"));
    run(
        &settings,
        &dunce::canonicalize(temp.path()).unwrap(),
        now + chrono::Duration::hours(1),
        || panic!("checked"),
    )
    .unwrap();
    assert!(error(&settings).unwrap().contains("did not finish"));
}

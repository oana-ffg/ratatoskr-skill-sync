use super::*;

#[test]
fn desktop_updates_need_the_app_beside_the_cli() {
    let temp = tempfile::tempdir().unwrap();
    let (dir, cli, desktop) = if cfg!(windows) {
        let dir = temp.path().join("Ratatoskr Skill Sync");
        (
            dir.clone(),
            dir.join("ratatoskr.exe"),
            dir.join("ratatoskr-desktop.exe"),
        )
    } else {
        let dir = temp.path().join("Ratatoskr Skill Sync.app/Contents/MacOS");
        (
            dir.clone(),
            dir.join("ratatoskr"),
            dir.join("ratatoskr-desktop"),
        )
    };
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&cli, b"").unwrap();
    assert!(desktop_beside(&cli).is_err());
    std::fs::write(&desktop, b"").unwrap();
    assert_eq!(desktop_beside(&cli).unwrap(), desktop);
    // A standalone CLI outside an app bundle is refused on macOS.
    let standalone = temp.path().join("bin/ratatoskr");
    std::fs::create_dir_all(standalone.parent().unwrap()).unwrap();
    std::fs::write(&standalone, b"").unwrap();
    assert!(desktop_beside(&standalone).is_err());
}

#[test]
fn desktop_requests_are_answered_once_and_settled_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let data = root.join("app");
    let app = root.join("ratatoskr-desktop");
    let other = root.join("backup/ratatoskr-desktop");
    let now = chrono::DateTime::parse_from_rfc3339("2026-10-27T20:00:00+01:00").unwrap();
    assert_eq!(pending_desktop_request(&data, &app).unwrap(), None);
    let first = request_desktop_update(&data, &app, now).unwrap();
    assert_eq!(
        pending_desktop_request(&data, &app).unwrap(),
        Some(first.clone())
    );
    // Another copy of the app never takes a request addressed to this one.
    assert_eq!(pending_desktop_request(&data, &other).unwrap(), None);
    answer_desktop_request(&data, &first, now, DesktopRequestOutcome::Current).unwrap();
    assert_eq!(pending_desktop_request(&data, &app).unwrap(), None);
    assert_eq!(
        finish_desktop_request(&data, &app, "0.2.17", now).unwrap(),
        None
    );

    // A new request is pending again; the old answer does not satisfy it.
    let second = request_desktop_update(&data, &app, now).unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(
        pending_desktop_request(&data, &app).unwrap(),
        Some(second.clone())
    );
    let installing = DesktopRequestOutcome::Installing {
        version: "0.2.17".into(),
    };
    answer_desktop_request(&data, &second, now, installing.clone()).unwrap();
    // A backup copy starting first never settles this copy's install, whatever its version.
    for version in ["0.2.17", "0.2.16"] {
        assert_eq!(
            finish_desktop_request(&data, &other, version, now).unwrap(),
            None
        );
    }
    assert_eq!(
        desktop_request_status(&data).unwrap().1.unwrap().outcome,
        installing
    );
    let installed = DesktopRequestOutcome::Installed {
        version: "0.2.17".into(),
    };
    assert_eq!(
        finish_desktop_request(&data, &app, "0.2.17", now).unwrap(),
        Some(installed.clone())
    );
    assert_eq!(
        desktop_request_status(&data).unwrap().1.unwrap().outcome,
        installed
    );

    // An installer that left the old version running is reported, not hidden.
    let third = request_desktop_update(&data, &app, now).unwrap();
    answer_desktop_request(&data, &third, now, installing).unwrap();
    let settled = finish_desktop_request(&data, &app, "0.2.16", now).unwrap();
    assert!(matches!(
        settled,
        Some(DesktopRequestOutcome::Failed { error }) if error.contains("still runs 0.2.16")
    ));
    assert_eq!(pending_desktop_request(&data, &app).unwrap(), None);
}

#[test]
fn the_running_desktop_holds_the_instance_and_names_its_executable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let data = root.join("app");
    let app = root.join("ratatoskr-desktop");
    assert_eq!(running_desktop(&data).unwrap(), None);
    let instance = claim_desktop_instance(&data, &app).unwrap().unwrap();
    assert_eq!(running_desktop(&data).unwrap(), Some(app.clone()));
    assert!(
        claim_desktop_instance(&data, &root.join("other"))
            .unwrap()
            .is_none()
    );
    drop(instance);
    assert_eq!(running_desktop(&data).unwrap(), None);
}
#[test]
fn desktop_install_defers_for_other_process_operations() {
    let temp = tempfile::tempdir().unwrap();
    let settings = crate::Settings {
        state_dir: temp.path().canonicalize().unwrap(),
        machine: "test".into(),
        ..Default::default()
    };
    for name in ["sync.lock", "apply.lock", "schedule.lock"] {
        let busy = crate::config::state_lock(&settings, name).unwrap();
        fs2::FileExt::try_lock_exclusive(&busy).unwrap();
        assert!(idle_guard(&settings, false).unwrap().is_none());
        fs2::FileExt::unlock(&busy).unwrap();
        drop(busy);
        let guard = idle_guard(&settings, false).unwrap().unwrap();
        let file = crate::config::state_lock(&settings, name).unwrap();
        assert!(fs2::FileExt::try_lock_exclusive(&file).is_err());
        let inherited = guard.locks[0].try_clone().unwrap();
        drop(guard);
        assert!(fs2::FileExt::try_lock_exclusive(&file).is_ok());
        fs2::FileExt::unlock(&file).unwrap();
        drop(inherited);
    }
}
#[test]
fn startup_install_waits_for_helper_work_to_finish() {
    let temp = tempfile::tempdir().unwrap();
    let settings = crate::Settings {
        state_dir: temp.path().canonicalize().unwrap(),
        machine: "test".into(),
        ..Default::default()
    };
    let busy = crate::config::state_lock(&settings, "sync.lock").unwrap();
    fs2::FileExt::try_lock_exclusive(&busy).unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let _guard = idle_guard(&settings, true).unwrap().unwrap();
        acquired_tx.send(()).unwrap();
    });
    started_rx.recv().unwrap();
    assert!(matches!(
        acquired_rx.recv_timeout(Duration::from_millis(50)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    fs2::FileExt::unlock(&busy).unwrap();
    acquired_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    worker.join().unwrap();
}

use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().mode() & 0o7777
}

#[test]
fn apply_and_deleted_restore_keep_private_modes() {
    for original in [0o400, 0o600, 0o640, 0o644, 0o700, 0o750, 0o755] {
        let f = Fixture::new();
        fs::create_dir_all(f.output()).unwrap();
        let path = f.output().join("extra.txt");
        fs::write(&path, "private original").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(original)).unwrap();
        let backup = ratatoskr_core::adopt(&f.settings).unwrap();
        apply(&f.settings).unwrap();
        assert!(!path.exists());
        ratatoskr_core::restore_backup(&f.settings, &backup).unwrap();
        assert_eq!(mode(&path), original);
        assert_eq!(fs::read(&path).unwrap(), b"private original");
    }
}

#[test]
fn updates_keep_private_mode_and_new_files_are_private() {
    let f = Fixture::new();
    f.write("skills/review-code/data.txt", "original");
    apply(&f.settings).unwrap();
    let path = f.output().join("data.txt");
    assert_eq!(mode(&path), 0o600);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    f.write("skills/review-code/data.txt", "updated");
    apply(&f.settings).unwrap();
    assert_eq!(mode(&path), 0o640);
}

#[test]
fn source_conflicts_and_reinstall_restore_keep_private_modes() {
    use ratatoskr_core::{ConflictAction, preview_source_edit, resolve_conflict, restore_backup};
    for action in [
        ConflictAction::CopyBack,
        ConflictAction::EditSource,
        ConflictAction::Reinstall,
    ] {
        let f = Fixture::new();
        f.write("skills/review-code/data.txt", "original");
        let source = f.settings.source.join("skills/review-code/data.txt");
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
        apply(&f.settings).unwrap();
        let path = f.output().join("data.txt");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, "installed edit").unwrap();
        let review = if action == ConflictAction::EditSource {
            serde_json::to_string(
                &preview_source_edit(&f.settings, &path, "edited source").unwrap(),
            )
            .unwrap()
        } else {
            reviewed_resolution(&f, &path, action)
        };
        let backup = resolve_conflict(&f.settings, &review).unwrap();
        assert_eq!(mode(&source), 0o600);
        assert_eq!(mode(&path), 0o600);
        // Source-only editing deliberately leaves installed drift unresolved.
        if action == ConflictAction::EditSource {
            let adopt = reviewed_resolution(&f, &path, ConflictAction::Adopt);
            resolve_conflict(&f.settings, &adopt).unwrap();
        }
        restore_backup(&f.settings, &backup).unwrap();
        assert_eq!(mode(&source), 0o600);
        assert_eq!(mode(&path), 0o600);
        assert_eq!(fs::read(&source).unwrap(), b"original");
    }
}

#[test]
fn restore_does_not_undo_later_tightening_and_legacy_defaults_are_private() {
    let f = Fixture::new();
    f.write("skills/review-code/data.txt", "original");
    apply(&f.settings).unwrap();
    let path = f.output().join("data.txt");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let backup = ratatoskr_core::adopt(&f.settings).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    ratatoskr_core::restore_backup(&f.settings, &backup).unwrap();
    assert_eq!(mode(&path), 0o600);
    let snapshot_path = backup.join("snapshot.json");
    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&fs::read(&snapshot_path).unwrap()).unwrap();
    snapshot.as_object_mut().unwrap().remove("protections");
    fs::write(&snapshot_path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    fs::remove_file(f.settings.source.join("skills/review-code/data.txt")).unwrap();
    apply(&f.settings).unwrap();
    ratatoskr_core::restore_backup(&f.settings, &backup).unwrap();
    assert_eq!(mode(&path), 0o600);
}

#[test]
fn execute_changes_never_add_read_access() {
    let f = Fixture::new();
    f.write("skills/review-code/run.sh", "#!/bin/sh\n");
    apply(&f.settings).unwrap();
    let source = f.settings.source.join("skills/review-code/run.sh");
    let path = f.output().join("run.sh");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
    apply(&f.settings).unwrap();
    assert_eq!(mode(&path), 0o700);
    fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).unwrap();
    apply(&f.settings).unwrap();
    assert_eq!(mode(&path), 0o600);
}

#[cfg(target_os = "macos")]
#[test]
fn group_identity_survives_update_and_deleted_restore() {
    let f = Fixture::new();
    f.write("skills/review-code/data.txt", "original");
    apply(&f.settings).unwrap();
    let path = f.output().join("data.txt");
    let parent_gid = fs::metadata(path.parent().unwrap()).unwrap().gid();
    let groups = Command::new("id").arg("-G").output().unwrap();
    assert!(groups.status.success());
    let group = String::from_utf8(groups.stdout)
        .unwrap()
        .split_whitespace()
        .map(|id| id.parse::<u32>().unwrap())
        .find(|id| *id != parent_gid)
        .expect("macOS probe requires an alternate group of the current user");
    assert!(
        Command::new("chgrp")
            .arg(group.to_string())
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let backup = ratatoskr_core::adopt(&f.settings).unwrap();
    f.write("skills/review-code/data.txt", "updated");
    apply(&f.settings).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().gid(), group);
    fs::remove_file(f.settings.source.join("skills/review-code/data.txt")).unwrap();
    apply(&f.settings).unwrap();
    ratatoskr_core::restore_backup(&f.settings, &backup).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().gid(), group);
    assert_eq!(mode(&path), 0o640);
}

#[cfg(target_os = "macos")]
fn deny_acl(path: &Path, inherited: bool) {
    let ace = if inherited {
        "user:nobody deny read,file_inherit,directory_inherit"
    } else {
        "user:nobody deny read"
    };
    assert!(
        Command::new("chmod")
            .args(["+a", ace])
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
}

#[cfg(target_os = "macos")]
#[test]
fn native_deny_acl_refuses_update_before_replacement() {
    let f = Fixture::new();
    f.write("skills/review-code/data.txt", "original");
    apply(&f.settings).unwrap();
    let path = f.output().join("data.txt");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    deny_acl(&path, false);
    f.write("skills/review-code/data.txt", "updated");
    assert!(apply(&f.settings).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"original");
    let acl = Command::new("ls").arg("-le").arg(&path).output().unwrap();
    assert!(String::from_utf8(acl.stdout).unwrap().contains("deny read"));
}

#[cfg(target_os = "macos")]
#[test]
fn inherited_deny_acl_refuses_new_output() {
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    deny_acl(&f.output(), true);
    f.write("skills/review-code/new.txt", "private new bytes");
    assert!(apply(&f.settings).is_err());
    assert!(!f.output().join("new.txt").exists());
}

#[cfg(target_os = "macos")]
#[test]
fn native_acls_refuse_conflicts_backups_deletion_and_restore() {
    use ratatoskr_core::{ConflictAction, preview_resolution, preview_source_edit};
    let f = Fixture::new();
    f.write("skills/review-code/data.txt", "original");
    apply(&f.settings).unwrap();
    let path = f.output().join("data.txt");
    let source = f.settings.source.join("skills/review-code/data.txt");
    let backup = ratatoskr_core::adopt(&f.settings).unwrap();
    fs::write(&path, "installed edit").unwrap();
    deny_acl(&source, false);
    assert!(preview_resolution(&f.settings, &path, ConflictAction::CopyBack).is_err());
    assert!(preview_source_edit(&f.settings, &path, "source edit").is_err());
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert!(
        Command::new("chmod")
            .arg("-N")
            .arg(&source)
            .status()
            .unwrap()
            .success()
    );
    deny_acl(&path, false);
    assert!(preview_resolution(&f.settings, &path, ConflictAction::Reinstall).is_err());
    assert!(ratatoskr_core::adopt(&f.settings).is_err());
    assert!(ratatoskr_core::restore_backup(&f.settings, &backup).is_err());
    fs::remove_file(&source).unwrap();
    f.write("skills/review-code/a-new.txt", "must not be written");
    assert!(apply(&f.settings).is_err());
    assert!(!f.output().join("a-new.txt").exists());
    assert_eq!(fs::read(&path).unwrap(), b"installed edit");
}

#[cfg(target_os = "macos")]
#[test]
fn new_inherited_restrictions_block_deleted_restore() {
    let f = Fixture::new();
    f.write("skills/review-code/data.txt", "original");
    apply(&f.settings).unwrap();
    let backup = ratatoskr_core::adopt(&f.settings).unwrap();
    fs::remove_file(f.settings.source.join("skills/review-code/data.txt")).unwrap();
    apply(&f.settings).unwrap();
    deny_acl(&f.output(), true);
    assert!(ratatoskr_core::restore_backup(&f.settings, &backup).is_err());
    assert!(!f.output().join("data.txt").exists());
}

#[cfg(target_os = "macos")]
#[test]
fn deny_delete_child_refuses_without_leaking_tempfiles() {
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    f.write("skills/review-code/new.txt", "new");
    let directory = f.output();
    let user = Command::new("id").arg("-un").output().unwrap();
    assert!(user.status.success());
    let ace = format!(
        "user:{} deny delete_child",
        String::from_utf8(user.stdout).unwrap().trim()
    );
    assert!(
        Command::new("chmod")
            .args(["+a", &ace])
            .arg(&directory)
            .status()
            .unwrap()
            .success()
    );
    let before: Vec<_> = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    let result = apply(&f.settings);
    let after: Vec<_> = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(
        Command::new("chmod")
            .arg("-N")
            .arg(&directory)
            .status()
            .unwrap()
            .success()
    );
    assert!(result.is_err());
    assert_eq!(before, after);
}

#[test]
fn source_editor_keeps_executable_and_private_mode() {
    let f = Fixture::new();
    let source = f.settings.source.join("skills/review-code/skill.toml");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
    let document = ratatoskr_core::read_skill(&f.settings, None, "review-code").unwrap();
    ratatoskr_core::write_skill_file(
        &f.settings,
        None,
        "review-code",
        "skill.toml",
        "# edited\n",
        &document.metadata_hash,
    )
    .unwrap();
    assert_eq!(mode(&source), 0o700);
}

#[test]
fn reviewed_apply_rejects_protection_drift() {
    let f = Fixture::new();
    f.write("skills/review-code/data.txt", "original");
    apply(&f.settings).unwrap();
    f.write("skills/review-code/data.txt", "updated");
    let preview = serde_json::to_string(&plan(&f.settings).unwrap()).unwrap();
    let path = f.output().join("data.txt");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    assert!(ratatoskr_core::apply_reviewed(&f.settings, &preview).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"original");
}

#[test]
fn foreign_identity_or_invalid_mode_in_backup_refuses_before_any_restore() {
    for (field, invalid) in [("gid", u32::MAX), ("uid", u32::MAX), ("mode", 0o10600)] {
        let f = Fixture::new();
        f.write("skills/review-code/z-data.txt", "original");
        apply(&f.settings).unwrap();
        let backup = ratatoskr_core::adopt(&f.settings).unwrap();
        let before = fs::read(f.output().join("SKILL.md")).unwrap();
        let before_inode = fs::metadata(f.output().join("SKILL.md")).unwrap().ino();
        f.write("skills/review-code/z-data.txt", "updated");
        apply(&f.settings).unwrap();
        let path = f.output().join("z-data.txt");
        let snapshot_path = backup.join("snapshot.json");
        let mut snapshot: serde_json::Value =
            serde_json::from_slice(&fs::read(&snapshot_path).unwrap()).unwrap();
        snapshot["protections"][path.to_str().unwrap()][field] = invalid.into();
        fs::write(&snapshot_path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        assert!(ratatoskr_core::restore_backup(&f.settings, &backup).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"updated");
        assert_eq!(fs::read(f.output().join("SKILL.md")).unwrap(), before);
        assert_eq!(
            fs::metadata(f.output().join("SKILL.md")).unwrap().ino(),
            before_inode
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn parent_inherited_group_needs_no_membership_for_repeat_writes_or_restore() {
    // /private/tmp normally has group wheel, outside a standard user's
    // groups. macOS assigns that group to new files without chown privilege.
    let f = Fixture::with_temp(tempfile::tempdir_in("/private/tmp").unwrap());
    f.write("skills/review-code/data.txt", "original");
    apply(&f.settings).unwrap();
    let path = f.output().join("data.txt");
    let group = fs::metadata(path.parent().unwrap()).unwrap().gid();
    let backup = ratatoskr_core::adopt(&f.settings).unwrap();
    f.write("skills/review-code/data.txt", "updated");
    apply(&f.settings).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().gid(), group);
    fs::remove_file(f.settings.source.join("skills/review-code/data.txt")).unwrap();
    apply(&f.settings).unwrap();
    ratatoskr_core::restore_backup(&f.settings, &backup).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().gid(), group);
    assert_eq!(mode(&path), 0o600);
}

#[test]
fn restore_keeps_later_execute_class_tightening() {
    let f = Fixture::new();
    f.write("skills/review-code/run.sh", "#!/bin/sh\n");
    fs::set_permissions(
        f.settings.source.join("skills/review-code/run.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    apply(&f.settings).unwrap();
    let path = f.output().join("run.sh");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    let backup = ratatoskr_core::adopt(&f.settings).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    ratatoskr_core::restore_backup(&f.settings, &backup).unwrap();
    assert_eq!(mode(&path), 0o700);
}

#[test]
fn incompatible_execute_intersection_refuses_before_any_restore() {
    let f = Fixture::new();
    f.write("skills/review-code/z-run.sh", "#!/bin/sh\n");
    fs::set_permissions(
        f.settings.source.join("skills/review-code/z-run.sh"),
        fs::Permissions::from_mode(0o750),
    )
    .unwrap();
    apply(&f.settings).unwrap();
    let path = f.output().join("z-run.sh");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
    let backup = ratatoskr_core::adopt(&f.settings).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o641)).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    let earlier = fs::metadata(f.output().join("SKILL.md")).unwrap().ino();
    let error = ratatoskr_core::restore_backup(&f.settings, &backup).unwrap_err();
    assert!(format!("{error:#}").contains("executable permission"));
    assert_eq!(mode(&path), 0o641);
    assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(
        fs::metadata(f.output().join("SKILL.md")).unwrap().ino(),
        earlier
    );
}

#[test]
fn legacy_restore_into_existing_public_destination_defaults_to_owner_only() {
    let f = Fixture::new();
    f.write("skills/review-code/data.txt", "private original");
    apply(&f.settings).unwrap();
    let backup = ratatoskr_core::adopt(&f.settings).unwrap();
    let snapshot_path = backup.join("snapshot.json");
    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&fs::read(&snapshot_path).unwrap()).unwrap();
    snapshot.as_object_mut().unwrap().remove("protections");
    fs::write(&snapshot_path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    f.write("skills/review-code/data.txt", "harmless replacement");
    apply(&f.settings).unwrap();
    let path = f.output().join("data.txt");
    for (current, expected) in [(0o644, 0o600), (0o440, 0o400)] {
        fs::set_permissions(&path, fs::Permissions::from_mode(current)).unwrap();
        ratatoskr_core::restore_backup(&f.settings, &backup).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"private original");
        assert_eq!(mode(&path), expected);
    }
}

#[test]
fn legacy_source_backups_never_republish_private_bytes_with_public_mode() {
    use ratatoskr_core::{ConflictAction, preview_source_edit, resolve_conflict};
    for action in [ConflictAction::CopyBack, ConflictAction::EditSource] {
        let f = Fixture::new();
        f.write("skills/review-code/data.txt", "private original");
        let source = f.settings.source.join("skills/review-code/data.txt");
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
        apply(&f.settings).unwrap();
        let path = f.output().join("data.txt");
        fs::write(&path, "harmless installed edit").unwrap();
        let review = if action == ConflictAction::EditSource {
            serde_json::to_string(
                &preview_source_edit(&f.settings, &path, "harmless source edit").unwrap(),
            )
            .unwrap()
        } else {
            reviewed_resolution(&f, &path, action)
        };
        let backup = resolve_conflict(&f.settings, &review).unwrap();
        if action == ConflictAction::EditSource {
            resolve_conflict(
                &f.settings,
                &reviewed_resolution(&f, &path, ConflictAction::Adopt),
            )
            .unwrap();
        }
        let snapshot_path = backup.join("snapshot.json");
        let mut snapshot: serde_json::Value =
            serde_json::from_slice(&fs::read(&snapshot_path).unwrap()).unwrap();
        snapshot.as_object_mut().unwrap().remove("protections");
        for source in snapshot["sources"].as_object_mut().unwrap().values_mut() {
            source.as_object_mut().unwrap().remove("protection");
        }
        fs::write(&snapshot_path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).unwrap();
        ratatoskr_core::restore_backup(&f.settings, &backup).unwrap();
        assert_eq!(fs::read(&source).unwrap(), b"private original");
        assert_eq!(mode(&source), 0o600);
    }
}

#[test]
fn incompatible_source_execute_intersection_refuses_before_mutation() {
    use ratatoskr_core::{ConflictAction, preview_source_edit, resolve_conflict};
    for action in [ConflictAction::CopyBack, ConflictAction::EditSource] {
        let f = Fixture::new();
        f.write("skills/review-code/run.sh", "#!/bin/sh\nexit 0\n");
        let source = f.settings.source.join("skills/review-code/run.sh");
        fs::set_permissions(&source, fs::Permissions::from_mode(0o750)).unwrap();
        apply(&f.settings).unwrap();
        let path = f.output().join("run.sh");
        fs::write(&path, "#!/bin/sh\nexit 1\n").unwrap();
        let review = if action == ConflictAction::EditSource {
            serde_json::to_string(
                &preview_source_edit(&f.settings, &path, "#!/bin/sh\nexit 2\n").unwrap(),
            )
            .unwrap()
        } else {
            reviewed_resolution(&f, &path, action)
        };
        let backup = resolve_conflict(&f.settings, &review).unwrap();
        if action == ConflictAction::EditSource {
            resolve_conflict(
                &f.settings,
                &reviewed_resolution(&f, &path, ConflictAction::Adopt),
            )
            .unwrap();
        }
        fs::set_permissions(&source, fs::Permissions::from_mode(0o641)).unwrap();
        let source_inode = fs::metadata(&source).unwrap().ino();
        let output_inode = fs::metadata(&path).unwrap().ino();
        let error = ratatoskr_core::restore_backup(&f.settings, &backup).unwrap_err();
        assert!(format!("{error:#}").contains("executable permission"));
        assert_eq!(mode(&source), 0o641);
        assert_eq!(fs::metadata(&source).unwrap().ino(), source_inode);
        assert_eq!(fs::metadata(&path).unwrap().ino(), output_inode);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_parent_access_acl_is_safe_but_default_and_file_acls_refuse() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    fn set_acl(path: &Path, attribute: &std::ffi::CStr, directory: bool) {
        // Linux UAPI posix_acl_xattr_header/entry, version2, little-endian.
        // Include a named user so the kernel retains an extended ACL.
        let named = if unsafe { libc::geteuid() } == 0 {
            1
        } else {
            0
        };
        let owner = if directory { 7 } else { 6 };
        let group = if directory { 5 } else { 4 };
        let mut bytes = 2_u32.to_le_bytes().to_vec();
        for (tag, permission, id) in [
            (1_u16, owner, u32::MAX),
            (2, 0, named),
            (4, group, u32::MAX),
            (16, group, u32::MAX),
            (32, 0, u32::MAX),
        ] {
            bytes.extend(tag.to_le_bytes());
            bytes.extend((permission as u16).to_le_bytes());
            bytes.extend(id.to_le_bytes());
        }
        let name = CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: valid terminated names and a live buffer of exactly len bytes.
        let result = unsafe {
            libc::setxattr(
                name.as_ptr(),
                attribute.as_ptr(),
                bytes.as_ptr().cast(),
                bytes.len(),
                0,
            )
        };
        assert_eq!(
            result,
            0,
            "Set Linux probe ACL: {}",
            std::io::Error::last_os_error()
        );
    }
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    set_acl(&f.output(), c"system.posix_acl_access", true);
    f.write("skills/review-code/data.txt", "original");
    apply(&f.settings).unwrap();
    let path = f.output().join("data.txt");
    set_acl(&path, c"system.posix_acl_access", false);
    f.write("skills/review-code/data.txt", "updated");
    assert!(apply(&f.settings).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"original");
    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: both attribute strings are terminated and name is live.
    assert_eq!(
        unsafe { libc::removexattr(name.as_ptr(), c"system.posix_acl_access".as_ptr()) },
        0
    );
    set_acl(&f.output(), c"system.posix_acl_default", true);
    f.write("skills/review-code/new.txt", "must not be written");
    assert!(apply(&f.settings).is_err());
    assert!(!f.output().join("new.txt").exists());
}

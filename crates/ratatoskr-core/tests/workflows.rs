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

#[cfg(unix)]
mod private_permissions {
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
        use ratatoskr_core::{
            ConflictAction, preview_source_edit, resolve_conflict, restore_backup,
        };
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
}

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

impl Fixture {
    fn new() -> Self {
        Self::with_temp(tempfile::tempdir().unwrap())
    }

    fn with_temp(temp: tempfile::TempDir) -> Self {
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
    let remote_url = remote
        .to_str()
        .unwrap()
        .trim_start_matches("\\\\?\\")
        .replace('\\', "/");
    git(&project, &["remote", "add", "origin", &remote_url]);
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
    let key = include_str!("fixtures/update.pub");
    let payload = include_bytes!("fixtures/update.txt");
    let signature = include_str!("fixtures/update.txt.sig");
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

fn reviewed_resolution(f: &Fixture, path: &Path, action: ratatoskr_core::ConflictAction) -> String {
    serde_json::to_string(&ratatoskr_core::preview_resolution(&f.settings, path, action).unwrap())
        .unwrap()
}

#[test]
fn conflict_reinstall_is_scoped_backed_up_and_restorable() {
    use ratatoskr_core::{ConflictAction, resolve_conflict, restore_backup};
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    let path = f.output().join("SKILL.md");
    fs::write(&path, "My hand edit").unwrap();
    f.write("skills/review-code/new.txt", "Unrelated addition");
    let check = plan(&f.settings).unwrap();
    assert_eq!(check.conflict_details[0].kind, "hand-edited");
    let preview = reviewed_resolution(&f, &path, ConflictAction::Reinstall);
    assert_eq!(fs::read_to_string(&path).unwrap(), "My hand edit");
    let backup = resolve_conflict(&f.settings, &preview).unwrap();
    assert!(backup.join("snapshot.json").is_file());
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("Read /knowledge")
    );
    assert!(!f.output().join("new.txt").exists());
    restore_backup(&f.settings, &backup).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), "My hand edit");
    assert_eq!(
        plan(&f.settings).unwrap().conflict_details[0].kind,
        "hand-edited"
    );
}

#[test]
fn removed_conflict_reinstall_and_restore_preserve_absence() {
    use ratatoskr_core::{ConflictAction, resolve_conflict, restore_backup};
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    let path = f.output().join("SKILL.md");
    fs::remove_file(&path).unwrap();
    assert_eq!(
        plan(&f.settings).unwrap().conflict_details[0].kind,
        "removed"
    );
    let backup = resolve_conflict(
        &f.settings,
        &reviewed_resolution(&f, &path, ConflictAction::Reinstall),
    )
    .unwrap();
    assert!(path.exists());
    restore_backup(&f.settings, &backup).unwrap();
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn permission_conflict_can_be_reinstalled_and_restored() {
    use ratatoskr_core::{ConflictAction, resolve_conflict, restore_backup};
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    let path = f.output().join("SKILL.md");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        plan(&f.settings).unwrap().conflict_details[0].kind,
        "permission-change"
    );
    let backup = resolve_conflict(
        &f.settings,
        &reviewed_resolution(&f, &path, ConflictAction::Reinstall),
    )
    .unwrap();
    assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o111, 0);
    restore_backup(&f.settings, &backup).unwrap();
    assert_ne!(fs::metadata(&path).unwrap().permissions().mode() & 0o111, 0);
}

#[test]
fn adoption_records_the_edit_as_is_and_keeps_source_changes_pending() {
    use ratatoskr_core::{ConflictAction, resolve_conflict};
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    let path = f.output().join("SKILL.md");
    fs::write(&path, "Keep my copy").unwrap();
    let backup = resolve_conflict(
        &f.settings,
        &reviewed_resolution(&f, &path, ConflictAction::Adopt),
    )
    .unwrap();
    assert!(backup.join("snapshot.json").exists());
    assert_eq!(fs::read_to_string(&path).unwrap(), "Keep my copy");
    let check = plan(&f.settings).unwrap();
    assert!(check.conflicts.is_empty());
    assert_eq!(check.changes.len(), 1);
}

#[test]
fn unmanaged_directory_resolution_backs_up_extra_files() {
    use ratatoskr_core::{ConflictAction, resolve_conflict, restore_backup};
    let f = Fixture::new();
    fs::create_dir_all(f.output()).unwrap();
    fs::write(f.output().join("personal.txt"), "Personal").unwrap();
    let preview = reviewed_resolution(&f, &f.output(), ConflictAction::Reinstall);
    assert!(preview.contains("personal.txt"));
    let backup = resolve_conflict(&f.settings, &preview).unwrap();
    assert!(plan(&f.settings).unwrap().conflicts.is_empty());
    assert!(!f.output().join("personal.txt").exists());
    restore_backup(&f.settings, &backup).unwrap();
    assert_eq!(
        fs::read_to_string(f.output().join("personal.txt")).unwrap(),
        "Personal"
    );
    assert!(!f.output().join("SKILL.md").exists());
}

#[test]
fn unmanaged_file_can_be_adopted_or_discarded_explicitly() {
    use ratatoskr_core::{ConflictAction, resolve_conflict};
    for action in [ConflictAction::Adopt, ConflictAction::Reinstall] {
        let f = Fixture::new();
        apply(&f.settings).unwrap();
        let path = f.output().join("personal.txt");
        fs::write(&path, "Personal").unwrap();
        assert_eq!(
            plan(&f.settings).unwrap().conflict_details[0].kind,
            "unmanaged-collision"
        );
        resolve_conflict(&f.settings, &reviewed_resolution(&f, &path, action)).unwrap();
        assert!(plan(&f.settings).unwrap().conflicts.is_empty());
        assert_eq!(path.exists(), action == ConflictAction::Adopt);
    }
}

#[test]
fn copy_back_updates_unambiguous_source_and_restore_recovers_both_sides() {
    use ratatoskr_core::{ConflictAction, resolve_conflict, restore_backup};
    let f = Fixture::new();
    let original = "---\nname: review-code\ndescription: Review changed code\n---\nOriginal.\n";
    f.write("skills/review-code/SKILL.md", original);
    apply(&f.settings).unwrap();
    let path = f.output().join("SKILL.md");
    let edited = original.replace("Original.", "My improvement.");
    fs::write(&path, &edited).unwrap();
    let backup = resolve_conflict(
        &f.settings,
        &reviewed_resolution(&f, &path, ConflictAction::CopyBack),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(f.settings.source.join("skills/review-code/SKILL.md")).unwrap(),
        edited
    );
    assert!(plan(&f.settings).unwrap().changes.is_empty());
    restore_backup(&f.settings, &backup).unwrap();
    assert_eq!(
        fs::read_to_string(f.settings.source.join("skills/review-code/SKILL.md")).unwrap(),
        original
    );
    assert_eq!(fs::read_to_string(path).unwrap(), edited);
    assert!(!plan(&f.settings).unwrap().conflicts.is_empty());
}

#[test]
fn ambiguous_or_invalid_copy_back_is_refused_without_writes() {
    use ratatoskr_core::{ConflictAction, preview_resolution, resolve_conflict};
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    let path = f.output().join("SKILL.md");
    fs::write(&path, "User edit").unwrap();
    assert!(preview_resolution(&f.settings, &path, ConflictAction::CopyBack).is_err());
    assert!(
        plan(&f.settings).unwrap().conflict_details[0]
            .copy_back_blocked
            .is_some()
    );
    assert!(!f.settings.state_dir.join("test/backups").exists());
    f.write(
        "skills/review-code/SKILL.md",
        "---\nname: review-code\ndescription: Review changed code\n---\nPlain.\n",
    );
    let review = reviewed_resolution(&f, &path, ConflictAction::CopyBack);
    assert!(
        resolve_conflict(&f.settings, &review).is_err(),
        "Invalid frontmatter must not enter source"
    );
    assert!(
        fs::read_to_string(f.settings.source.join("skills/review-code/SKILL.md"))
            .unwrap()
            .contains("Plain.")
    );
}

#[test]
fn changed_output_or_source_invalidates_resolution_preview() {
    use ratatoskr_core::{ConflictAction, resolve_conflict};
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    let path = f.output().join("SKILL.md");
    fs::write(&path, "First edit").unwrap();
    let review = reviewed_resolution(&f, &path, ConflictAction::Reinstall);
    fs::write(&path, "Second edit").unwrap();
    assert!(resolve_conflict(&f.settings, &review).is_err());
    let review = reviewed_resolution(&f, &path, ConflictAction::Reinstall);
    f.write(
        "skills/review-code/SKILL.md",
        "---\nname: review-code\ndescription: Review changed code\n---\nNew source.\n",
    );
    assert!(resolve_conflict(&f.settings, &review).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "Second edit");
}

#[test]
fn draft_preview_uses_deployment_renderer_without_writing_source_or_outputs() {
    let f = Fixture::new();
    let document = ratatoskr_core::read_skill(&f.settings, None, "review-code").unwrap();
    let files = ratatoskr_core::preview_skill(
        &f.settings,
        None,
        "review-code",
        &document.markdown,
        &document.metadata,
        "test",
        "codex",
    )
    .unwrap();
    assert_eq!(files.len(), 1);
    let text = files[0].text.as_ref().unwrap();
    assert!(text.contains("Read /knowledge"));
    assert!(!text.contains("{{af:"));
    assert!(!f.settings.state_dir.exists());
    apply(&f.settings).unwrap();
    assert_eq!(
        *text,
        fs::read_to_string(f.output().join("SKILL.md")).unwrap()
    );
    assert!(
        ratatoskr_core::preview_skill(&f.settings, None, "review-code", "bad", "", "test", "codex")
            .is_err()
    );
    assert!(
        ratatoskr_core::preview_skill(
            &f.settings,
            None,
            "review-code",
            &document.markdown,
            "",
            "test",
            "absent"
        )
        .is_err()
    );
}

#[test]
fn every_verified_preset_has_valid_source_definitions_and_real_documentation() {
    let f = Fixture::new();
    for mut preset in ratatoskr_core::agent_presets() {
        preset.target.tag = format!("preset-{}", preset.target.tag);
        assert_eq!(preset.sources.len(), 2);
        assert!(preset.sources.iter().all(|url| url.starts_with("https://")));
        let draft = ratatoskr_core::preview_agent_target(&f.settings, &preset.target).unwrap();
        ratatoskr_core::write_source_file(&f.settings, "harnesses.toml", &draft.text, &draft.hash)
            .unwrap();
        let profile = format!(
            "os='macos'\nmode='staging'\nharnesses=['{}']\n[paths]\nvault='/knowledge'\n",
            preset.target.tag
        );
        f.write("machines/test.toml", &profile);
        // The initial fixture excludes an undefined variable for Claude only; use plain content here.
        f.write(
            "skills/review-code/SKILL.md",
            "---\nname: review-code\ndescription: Review changed code\n---\nReview it.\n",
        );
        assert!(plan(&f.settings).is_ok(), "{}", preset.title);
    }
    let codex = ratatoskr_core::agent_presets()
        .into_iter()
        .find(|p| p.target.tag == "codex")
        .unwrap();
    assert_eq!(codex.target.global_folder, "{{af:home}}/.agents");
    assert_eq!(
        codex.target.instructions_folder.as_deref(),
        Some("{{af:home}}/.codex")
    );
}

#[test]
fn preset_form_refuses_duplicate_tags_and_unsafe_paths() {
    let f = Fixture::new();
    let mut target = ratatoskr_core::agent_presets().remove(0).target;
    target.tag = "codex".into();
    assert!(ratatoskr_core::preview_agent_target(&f.settings, &target).is_err());
    target.tag = "custom".into();
    target.project_folder = "../escape".into();
    assert!(ratatoskr_core::preview_agent_target(&f.settings, &target).is_err());
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
    assert!(status.log.contains("Sync failed"));
}

#[test]
fn ambiguous_conflict_can_be_edited_in_source_with_a_review_and_backup() {
    use ratatoskr_core::{preview_source_edit, read_conflict_source, resolve_conflict};
    let f = Fixture::new();
    apply(&f.settings).unwrap();
    let path = f.output().join("SKILL.md");
    fs::write(&path, "A hand edit").unwrap();
    let document = read_conflict_source(&f.settings, &path).unwrap();
    assert!(document.text.contains("{{af:paths.vault}}"));
    let text = document
        .text
        .replace("Report findings.", "Report precise findings.");
    let preview = preview_source_edit(&f.settings, &path, &text).unwrap();
    assert!(
        preview.effects[0]
            .diff
            .as_ref()
            .unwrap()
            .contains("Report precise findings")
    );
    let backup = resolve_conflict(&f.settings, &serde_json::to_string(&preview).unwrap()).unwrap();
    assert!(backup.join("source").exists());
    assert_eq!(read_conflict_source(&f.settings, &path).unwrap().text, text);
    assert_eq!(fs::read_to_string(path).unwrap(), "A hand edit");
    assert!(
        !plan(&f.settings).unwrap().conflicts.is_empty(),
        "Editing source does not implicitly adopt output drift"
    );
}

#[test]
fn preview_preserves_live_mode_context_without_touching_live_destinations() {
    let f = Fixture::new();
    f.write(
        "skills/review-code/SKILL.md",
        "---\nname: review-code\ndescription: Review changed code\n---\nMode: {{af:mode}}.\n",
    );
    f.write(
        "machines/test.toml",
        "os='windows'\nmode='live'\nharnesses=['codex']\n",
    );
    let doc = ratatoskr_core::read_skill(&f.settings, None, "review-code").unwrap();
    let preview = ratatoskr_core::preview_skill(
        &f.settings,
        None,
        "review-code",
        &doc.markdown,
        &doc.metadata,
        "test",
        "codex",
    )
    .unwrap();
    assert!(preview[0].text.as_ref().unwrap().contains("Mode: live."));
    assert!(!f.settings.state_dir.exists());
    assert!(
        plan(&f.settings).is_err(),
        "Live deployment still enforces the home and OS checks"
    );
}

#[test]
fn global_instruction_root_is_independent_of_the_skill_root_and_checked_for_overlap() {
    let f = Fixture::new();
    let skill_root = f.root.join("agent-home");
    let instructions_root = f.root.join("instructions-home");
    let definition = format!(
        "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\nglobal_root={}\n[codex.roots]\n{}={}\n",
        toml::Value::String(instructions_root.to_string_lossy().into()),
        std::env::consts::OS,
        toml::Value::String(skill_root.to_string_lossy().into())
    );
    f.write("harnesses.toml", &definition);
    let home = directories::BaseDirs::new()
        .unwrap()
        .home_dir()
        .canonicalize()
        .unwrap();
    f.write(
        "machines/test.toml",
        &format!(
            "os='{}'\nmode='live'\nharnesses=['codex']\nhome={}\n[paths]\nvault='/knowledge'\n",
            std::env::consts::OS,
            toml::Value::String(home.to_string_lossy().into())
        ),
    );
    f.write("global/AGENTS.md", "Global instructions.");
    apply(&f.settings).unwrap();
    assert!(skill_root.join("skills/review-code/SKILL.md").is_file());
    assert!(instructions_root.join("AGENTS.md").is_file());
    assert!(!skill_root.join("AGENTS.md").exists());
    f.write(
        "harnesses.toml",
        &definition.replace(
            &instructions_root.to_string_lossy().to_string(),
            &f.settings.source.to_string_lossy(),
        ),
    );
    assert!(plan(&f.settings).is_err());
}

#[test]
fn skill_analysis_returns_parser_ranges_in_utf16_and_resolved_values() {
    let f = Fixture::new();
    let markdown = "---\nname: review-code\ndescription: Review changed code\n---\n🌳 Café.\n<!-- only harness=codex -->Read {{af:paths.vault}}.<!-- /only -->\n";
    let analysis =
        ratatoskr_core::analyze_skill(&f.settings, "review-code", markdown, "test", "codex")
            .unwrap();
    assert!(analysis.diagnostics.is_empty());
    assert_eq!(analysis.blocks.len(), 1);
    let block = &analysis.blocks[0];
    assert!(block.active && block.reachable);
    let utf16: Vec<u16> = markdown.encode_utf16().collect();
    assert!(
        String::from_utf16(&utf16[block.opening.from..block.opening.to])
            .unwrap()
            .starts_with("<!-- only")
    );
    let variable = &analysis.variables[0];
    assert_eq!(
        String::from_utf16(&utf16[variable.range.from..variable.range.to]).unwrap(),
        "{{af:paths.vault}}"
    );
    assert_eq!(variable.value.as_deref(), Some("/knowledge"));
    assert!(!f.settings.state_dir.exists());
}

#[test]
fn skill_analysis_reports_dead_sections_unknown_variables_and_frontmatter_errors() {
    let f = Fixture::new();
    let markdown = "---\nname: review-code\ndescription: Review changed code\n---\n<!-- only harness=absent -->{{af:unknown}}<!-- /only -->\n{{af:missing}}";
    let analysis =
        ratatoskr_core::analyze_skill(&f.settings, "review-code", markdown, "test", "codex")
            .unwrap();
    assert!(!analysis.blocks[0].reachable);
    assert!(!analysis.variables[0].active);
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|d| d.severity == "warning" && d.message.contains("no enabled"))
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|d| d.message.contains("missing"))
    );
    assert!(
        !analysis
            .diagnostics
            .iter()
            .any(|d| d.message.contains("unknown"))
    );
    let markdown = format!(
        "---\nname: wrong-name\ndescription: {}\n---\nBody.\n",
        "a".repeat(1025)
    );
    let analysis =
        ratatoskr_core::analyze_skill(&f.settings, "review-code", &markdown, "test", "codex")
            .unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|d| d.message.contains("name must match"))
    );
    let markdown = markdown.replace("wrong-name", "review-code");
    let analysis =
        ratatoskr_core::analyze_skill(&f.settings, "review-code", &markdown, "test", "codex")
            .unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|d| d.message.contains("description"))
    );
}

#[test]
fn skill_analysis_and_rendering_agree_on_invalid_only_blocks() {
    let f = Fixture::new();
    for body in [
        "<!-- only harness=codex -->Unclosed",
        "<!-- /only -->Unexpected",
        "<!-- only harness=codex --><!-- only os=macos -->Nested<!-- /only --><!-- /only -->",
        "<!-- only wrong=value -->Unknown<!-- /only -->",
    ] {
        let markdown =
            format!("---\nname: review-code\ndescription: Review changed code\n---\n{body}\n");
        let analysis =
            ratatoskr_core::analyze_skill(&f.settings, "review-code", &markdown, "test", "codex")
                .unwrap();
        assert!(analysis.diagnostics.iter().any(|d| d.severity == "error"));
        assert!(
            ratatoskr_core::preview_skill(
                &f.settings,
                None,
                "review-code",
                &markdown,
                "",
                "test",
                "codex"
            )
            .is_err()
        );
    }
}

#[test]
fn restoring_copy_back_cannot_overwrite_later_source_edits() {
    use ratatoskr_core::{ConflictAction, resolve_conflict, restore_backup};
    let f = Fixture::new();
    let source = f.settings.source.join("skills/review-code/SKILL.md");
    f.write(
        "skills/review-code/SKILL.md",
        "---\nname: review-code\ndescription: Review changed code\n---\nOriginal.\n",
    );
    apply(&f.settings).unwrap();
    let path = f.output().join("SKILL.md");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("Original.", "Installed edit."),
    )
    .unwrap();
    let backup = resolve_conflict(
        &f.settings,
        &reviewed_resolution(&f, &path, ConflictAction::CopyBack),
    )
    .unwrap();
    let later = fs::read_to_string(&source)
        .unwrap()
        .replace("Installed edit.", "Later source edit.");
    fs::write(&source, &later).unwrap();
    assert!(
        restore_backup(&f.settings, &backup)
            .unwrap_err()
            .to_string()
            .contains("Source changed")
    );
    assert_eq!(fs::read_to_string(source).unwrap(), later);
    assert!(
        fs::read_to_string(path)
            .unwrap()
            .contains("Installed edit.")
    );
}

#[test]
fn adding_agent_definition_preserves_existing_text_and_comments() {
    let f = Fixture::new();
    let path = f.settings.source.join("harnesses.toml");
    let original = format!(
        "# Preserve this choice.\n{}\n# End of existing definitions.\n",
        fs::read_to_string(&path).unwrap()
    );
    fs::write(&path, &original).unwrap();
    let target = ratatoskr_core::agent_presets().remove(0).target;
    let preview = ratatoskr_core::preview_agent_target(&f.settings, &target).unwrap();
    assert!(preview.text.starts_with(&original));
    ratatoskr_core::add_agent_target(&f.settings, &target, &preview.hash).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), preview.text);
}

#[test]
fn structured_combined_skill_saves_validate_both_files_and_reject_stale_hashes() {
    let f = Fixture::new();
    let before = ratatoskr_core::read_skill(&f.settings, None, "review-code").unwrap();
    let changed = before
        .markdown
        .replace("Review changed code", "Review all changed code carefully");
    let metadata = "# keep this comment\n[target]\nharnesses = ['codex']\n";
    ratatoskr_core::save_skill_draft(
        &f.settings,
        None,
        "review-code",
        &changed,
        metadata,
        &before.markdown_hash,
        &before.metadata_hash,
    )
    .unwrap();
    let after = ratatoskr_core::read_skill(&f.settings, None, "review-code").unwrap();
    assert_eq!(after.markdown, changed);
    assert_eq!(after.metadata, metadata);
    assert!(
        ratatoskr_core::save_skill_draft(
            &f.settings,
            None,
            "review-code",
            &before.markdown,
            &before.metadata,
            &before.markdown_hash,
            &before.metadata_hash
        )
        .is_err()
    );
    assert!(
        ratatoskr_core::save_skill_draft(
            &f.settings,
            None,
            "review-code",
            "invalid yaml",
            "enabled=false",
            &after.markdown_hash,
            &after.metadata_hash
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(f.settings.source.join("skills/review-code/skill.toml")).unwrap(),
        metadata
    );
    assert_eq!(
        fs::read_to_string(f.settings.source.join("skills/review-code/SKILL.md")).unwrap(),
        changed
    );
}

#[test]
fn structured_machine_and_local_settings_keep_comments_and_refuse_concurrent_changes() {
    let f = Fixture::new();
    let file = "machines/test.toml";
    let path = f.settings.source.join(file);
    let old = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("# Keep profile comment\n{old}")).unwrap();
    let mut doc = ratatoskr_core::structured_source(&f.settings, file).unwrap();
    doc.values.insert(
        "capabilities".into(),
        toml::Value::try_from(vec!["gopass"]).unwrap(),
    );
    ratatoskr_core::save_structured_source(&f.settings, file, &doc.hash, doc.values.clone())
        .unwrap();
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.starts_with("# Keep profile comment\n"));
    assert!(saved.contains("[paths]\nvault='/knowledge'"));
    assert!(
        ratatoskr_core::save_structured_source(&f.settings, file, &doc.hash, doc.values).is_err()
    );
    let config = f.root.join("config.toml");
    save(&config, &f.settings).unwrap();
    let original = format!("# Local settings\n{}", fs::read_to_string(&config).unwrap());
    fs::write(&config, &original).unwrap();
    let mut local = ratatoskr_core::local_configuration(&config).unwrap();
    local.settings.updates.time = "03:15".into();
    ratatoskr_core::save_local_configuration(&config, &local.settings, &local.hash).unwrap();
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        original.replace("00:00", "03:15")
    );
    assert!(
        ratatoskr_core::save_local_configuration(&config, &local.settings, &local.hash).is_err()
    );
}

#[test]
fn preview_line_alignment_does_not_mark_identical_text_after_omitted_conditions() {
    assert_eq!(
        ratatoskr_core::different_preview_lines(
            "same\nagent specific\nlast\n",
            &["same\nlast\n".into()]
        ),
        vec![1]
    );
}

#[test]
fn rename_updates_variant_identities_and_keeps_resources_and_metadata() {
    let f = Fixture::new();
    f.write(
        "skills/review-code/variants/os=windows/SKILL.md",
        "---\nname: review-code\ndescription: Windows guidance\n---\nWindows steps\n",
    );
    f.write("skills/review-code/resource.txt", "untouched bytes\r\n");
    let before = ratatoskr_core::read_skill(&f.settings, None, "review-code").unwrap();
    ratatoskr_core::rename_skill(
        &f.settings,
        None,
        "review-code",
        "review-changes",
        &before.markdown_hash,
    )
    .unwrap();
    assert!(!f.settings.source.join("skills/review-code").exists());
    assert_eq!(
        fs::read_to_string(f.settings.source.join("skills/review-changes/resource.txt")).unwrap(),
        "untouched bytes\r\n"
    );
    assert!(
        fs::read_to_string(
            f.settings
                .source
                .join("skills/review-changes/variants/os=windows/SKILL.md")
        )
        .unwrap()
        .contains("name: \"review-changes\"")
    );
    assert_eq!(
        fs::read_to_string(f.settings.source.join("skills/review-changes/skill.toml")).unwrap(),
        before.metadata
    );
}

#[test]
fn legacy_alert_settings_save_as_references_without_duplicate_keys() {
    let f = Fixture::new();
    let config = f.root.join("config.toml");
    save(&config, &f.settings).unwrap();
    let prefix = format!(
        "# Preserve local settings\n{}",
        fs::read_to_string(&config).unwrap()
    );
    fs::write(
        &config,
        format!(
            "{prefix}\n[alert]\nkind='pushover'\ntoken_env='PUSH_TOKEN'\nuser_env='PUSH_USER'\n"
        ),
    )
    .unwrap();
    let mut local = ratatoskr_core::local_configuration(&config).unwrap();
    local.settings.alert = Some(ratatoskr_core::AlertSettings::Pushover {
        token: ratatoskr_core::SecretReference::Gopass {
            entry: "pushover/codex-approvals/app-token".into(),
        },
        user: ratatoskr_core::SecretReference::Gopass {
            entry: "pushover/codex-approvals/user-key".into(),
        },
    });
    ratatoskr_core::save_local_configuration(&config, &local.settings, &local.hash).unwrap();
    let stored = fs::read_to_string(&config).unwrap();
    assert!(stored.starts_with(&prefix));
    assert!(!stored.contains("token_env") && !stored.contains("user_env"));
    let roundtrip = load(&config).unwrap();
    assert_eq!(
        serde_json::to_value(roundtrip.alert).unwrap(),
        serde_json::to_value(local.settings.alert).unwrap()
    );
    let local = ratatoskr_core::local_configuration(&config).unwrap();
    ratatoskr_core::save_local_configuration(&config, &local.settings, &local.hash).unwrap();
    assert_eq!(fs::read_to_string(config).unwrap(), stored);
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
    assert!(status.log.contains("sync.stdout.log"));
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
    assert!(status.log.contains("sync.stderr.log"));
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
fn source_validation_renders_all_profiles_without_writing_anything() {
    let f = Fixture::new();
    f.write("machines/remote.toml", "os='windows'\nmode='live'\nhome='Z:\\remote-user'\nharnesses=['codex']\n[paths]\nvault='Z:\\knowledge'\n");
    f.write(
        "global/AGENTS.md",
        "Use {{af:machine}} with {{af:harness}}.\n",
    );
    fn snapshot(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
        let mut entries = Vec::new();
        for entry in walkdir::WalkDir::new(root) {
            let entry = entry.unwrap();
            entries.push((
                entry.path().strip_prefix(root).unwrap().into(),
                entry
                    .file_type()
                    .is_file()
                    .then(|| fs::read(entry.path()).unwrap()),
            ));
        }
        entries.sort();
        entries
    }
    let before = snapshot(&f.root);
    let report = ratatoskr_core::validate_source(&f.settings.source).unwrap();
    assert!(report.problems.is_empty(), "{:?}", report.problems);
    assert_eq!(
        report
            .profiles
            .iter()
            .map(|p| p.machine.as_str())
            .collect::<Vec<_>>(),
        ["remote", "test"]
    );
    for profile in &report.profiles {
        assert_eq!(profile.harnesses.len(), 1);
        assert_eq!(profile.harnesses[0].skills_rendered, 1);
        assert!(profile.harnesses[0].global_instructions_rendered);
    }
    assert_eq!(
        snapshot(&f.root),
        before,
        "Validation must not change source or create state/staging"
    );
}

#[test]
fn source_validation_collects_remote_templates_files_skills_globals_and_profiles() {
    let f = Fixture::new();
    f.write("machines/remote.toml", "os='windows'\nmode='live'\nhome='Z:\\remote-user'\nharnesses=['codex', 'unknown']\n[paths]\nvault='Z:\\knowledge'\n");
    f.write("machines/broken.toml", "os = [");
    f.write("skills/review-code/SKILL.md", "---\nname: review-code\ndescription: Review code\n---\nGood locally.\n<!-- only machine=remote -->\n{{af:missing_remote}}\n<!-- /only -->\n");
    f.write("skills/review-code/extra.txt", "{{af:missing_file}}");
    f.write("skills/second/skill.toml", "[target]\ninvalid=['codex']\n");
    f.write(
        "skills/second/SKILL.md",
        "Not reached because targeting is invalid",
    );
    f.write(
        "global/AGENTS.md",
        "<!-- only machine=remote -->\n{{af:missing_global}}\n<!-- /only -->\n",
    );
    let report = ratatoskr_core::validate_source(&f.settings.source).unwrap();
    assert_eq!(report.profiles.len(), 3);
    let remote = report
        .problems
        .iter()
        .filter(|p| p.contexts.iter().any(|c| c.machine == "remote"))
        .collect::<Vec<_>>();
    assert_eq!(remote.len(), 5, "{remote:?}");
    assert!(
        remote.iter().any(|p| p
            .contexts
            .iter()
            .any(|c| c.harness.as_deref() == Some("codex")
                && c.skill.as_deref() == Some("review-code"))
            && p.path == "skills/review-code/SKILL.md"
            && p.message.contains("missing_remote"))
    );
    assert!(
        remote
            .iter()
            .any(|p| p.path == "skills/review-code/extra.txt" && p.message.contains("missing_file"))
    );
    assert!(remote.iter().any(|p| {
        p.contexts
            .iter()
            .any(|c| c.skill.as_deref() == Some("second"))
            && p.message.contains("Unknown target selector")
    }));
    assert!(
        remote
            .iter()
            .any(|p| p.contexts.iter().any(|c| c.skill.is_none())
                && p.path == "global/AGENTS.md"
                && p.message.contains("missing_global"))
    );
    assert!(remote.iter().any(|p| {
        p.contexts
            .iter()
            .any(|c| c.harness.as_deref() == Some("unknown"))
    }));
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.path == "machines/broken.toml"
                && p.contexts.iter().any(|c| c.machine == "broken"))
    );
    assert_eq!(
        report
            .profiles
            .iter()
            .find(|p| p.machine == "test")
            .unwrap()
            .problem_occurrences,
        2
    );
    assert!(
        plan(&f.settings).is_err(),
        "Sync/plan must still refuse bad sources"
    );
    assert!(!f.settings.state_dir.exists());
}

#[test]
fn source_validation_reports_invalid_variants_and_reserved_names() {
    let f = Fixture::new();
    f.write(
        "skills/review-code/variants/flavour=codex/readme.txt",
        "Invalid variant key",
    );
    f.write("skills/synced/skill.toml", "");
    f.write(
        "skills/synced/SKILL.md",
        "---\nname: synced\ndescription: Reserved name\n---\nBody\n",
    );
    let report = ratatoskr_core::validate_source(&f.settings.source).unwrap();
    assert_eq!(report.problems.len(), 2, "{:?}", report.problems);
    assert!(report.problems.iter().any(|p| {
        p.contexts
            .iter()
            .any(|c| c.skill.as_deref() == Some("review-code"))
            && p.message.contains("Unknown variant key")
    }));
    assert!(report.problems.iter().any(|p| {
        p.contexts
            .iter()
            .any(|c| c.skill.as_deref() == Some("synced"))
            && p.message.contains("Reserved skill name")
    }));
}

#[test]
fn source_validation_checks_foreign_live_root_templates_without_accessing_them() {
    let f = Fixture::new();
    f.write("machines/remote.toml", "os='windows'\nmode='live'\nhome='Z:\\remote-user'\nharnesses=['codex']\n[paths]\nvault='Z:\\knowledge'\n");
    f.write("harnesses.toml", "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\nglobal_root='relative'\n[codex.roots]\nwindows='{{af:missing_root}}'\n");
    let report = ratatoskr_core::validate_source(&f.settings.source).unwrap();
    assert_eq!(report.problems.len(), 2, "{:?}", report.problems);
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.message.contains("missing_root"))
    );
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.message.contains("must be absolute for windows"))
    );
    assert_eq!(
        report.profiles[0].harnesses[0].skills_rendered, 1,
        "Root errors must not hide template errors"
    );
}

#[test]
fn source_validation_reports_each_invalid_harness_and_continues_valid_targets() {
    let f = Fixture::new();
    f.write(
        "machines/test.toml",
        "os='macos'\nharnesses=['bad', 'codex', 'unknown']\n[paths]\nvault='/knowledge'\n",
    );
    let mut harnesses = fs::read_to_string(f.settings.source.join("harnesses.toml")).unwrap();
    harnesses.push_str(
        "\n[bad]\nskills_dir='../skills'\nglobal_file='AGENTS.md'\n[bad.roots]\nmacos='/unused'\n",
    );
    f.write("harnesses.toml", &harnesses);
    let report = ratatoskr_core::validate_source(&f.settings.source).unwrap();
    assert_eq!(report.problems.len(), 2, "{:?}", report.problems);
    assert!(report.problems.iter().all(|p| p.path == "harnesses.toml"));
    assert!(report.problems.iter().any(|p| {
        p.contexts
            .iter()
            .any(|c| c.harness.as_deref() == Some("bad"))
            && p.message.contains("Unsafe filename")
    }));
    assert!(report.problems.iter().any(|p| {
        p.contexts
            .iter()
            .any(|c| c.harness.as_deref() == Some("unknown"))
            && p.message.contains("Unknown harness")
    }));
    assert_eq!(report.profiles[0].harnesses.len(), 1);
    assert_eq!(report.profiles[0].harnesses[0].harness, "codex");
    assert_eq!(report.profiles[0].harnesses[0].skills_rendered, 1);
}

#[test]
fn source_validation_groups_shared_problems_with_every_context() {
    let f = Fixture::new();
    f.write("harnesses.toml", "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\n[codex.roots]\nmacos='/unused'\n[claude]\nskills_dir='skills'\nglobal_file='CLAUDE.md'\n[claude.roots]\nmacos='/unused'\n");
    for machine in ["first", "second", "test"] {
        f.write(
            &format!("machines/{machine}.toml"),
            "os='macos'\nharnesses=['codex', 'claude']\n",
        );
    }
    f.write(
        "skills/review-code/variants/flavour=codex/readme.txt",
        "Bad variant key",
    );
    f.write("skills/missing/skill.toml", "");
    f.write("skills/malformed/skill.toml", "target = [");
    let report = ratatoskr_core::validate_source(&f.settings.source).unwrap();
    assert_eq!(report.problems.len(), 3, "{:?}", report.problems);
    assert_eq!(
        report
            .problems
            .iter()
            .map(|p| p.path.as_str())
            .collect::<Vec<_>>(),
        ["skills/malformed", "skills/missing", "skills/review-code"]
    );
    for problem in &report.problems {
        let expected = ["first", "second", "test"]
            .into_iter()
            .flat_map(|machine| ["claude", "codex"].map(move |harness| (machine, harness)))
            .collect::<Vec<_>>();
        assert_eq!(
            problem
                .contexts
                .iter()
                .map(|c| (c.machine.as_str(), c.harness.as_deref().unwrap()))
                .collect::<Vec<_>>(),
            expected
        );
        assert!(problem.contexts.iter().all(|c| c.skill.is_some()));
    }
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.message.contains("Unknown variant key"))
    );
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.message.contains("lacks SKILL.md"))
    );
    assert!(report.problems.iter().any(|p| {
        p.contexts
            .iter()
            .any(|c| c.skill.as_deref() == Some("malformed"))
    }));
    for profile in &report.profiles {
        assert_eq!(
            profile.problem_occurrences, 6,
            "Counts must retain every failing target"
        );
        assert_eq!(profile.harnesses.len(), 2);
        assert!(profile.harnesses.iter().all(|h| h.skills_rendered == 0));
    }
    assert_eq!(report.problem_occurrences, 18);
    assert_eq!(
        report.problem_occurrences,
        report
            .profiles
            .iter()
            .map(|p| p.problem_occurrences)
            .sum::<usize>()
    );
    assert!(!f.settings.state_dir.exists());
}

#[test]
fn source_validation_keeps_distinct_messages_at_the_same_path() {
    let f = Fixture::new();
    f.write("machines/remote.toml", "os='macos'\nharnesses=['codex']\n");
    f.write("skills/review-code/SKILL.md", "---\nname: review-code\ndescription: Review code\n---\n<!-- only machine=remote -->{{af:missing_remote}}<!-- /only -->\n<!-- only machine=test -->{{af:missing_test}}<!-- /only -->\n");
    let report = ratatoskr_core::validate_source(&f.settings.source).unwrap();
    assert_eq!(report.problems.len(), 2);
    assert_eq!(report.problems[0].path, "skills/review-code/SKILL.md");
    assert_eq!(report.problems[0].path, report.problems[1].path);
    assert_ne!(report.problems[0].message, report.problems[1].message);
}

#[test]
fn deployment_propagates_render_errors_unchanged_without_writes() {
    for (path, content, expected) in [
        (
            "skills/review-code/skill.toml",
            "[target]\ninvalid=['codex']",
            "Unknown target selector",
        ),
        (
            "skills/review-code/extra.txt",
            "{{af:missing_file}}",
            "missing_file",
        ),
        (
            "global/AGENTS.md",
            "{{af:missing_global}}",
            "missing_global",
        ),
        (
            "harnesses.toml",
            "[codex]\nskills_dir='../skills'\nglobal_file='AGENTS.md'\n[codex.roots]\nmacos='/unused'\n",
            "Unsafe filename",
        ),
    ] {
        let f = Fixture::new();
        f.write(path, content);
        let report = ratatoskr_core::validate_source(&f.settings.source).unwrap();
        assert_eq!(report.problems.len(), 1, "{path}: {:?}", report.problems);
        assert!(report.problems[0].message.contains(expected));
        let error = format!("{:#}", plan(&f.settings).unwrap_err());
        assert_eq!(error, report.problems[0].message, "{path}");
        assert!(!f.settings.state_dir.exists());
    }
}

#[test]
fn source_validation_labels_non_string_harnesses_with_the_profile_and_value() {
    let f = Fixture::new();
    f.write(
        "machines/test.toml",
        "os='macos'\nharnesses=[42, 'codex']\n[paths]\nvault='/knowledge'\n",
    );
    let report = ratatoskr_core::validate_source(&f.settings.source).unwrap();
    assert_eq!(report.problems.len(), 1);
    let problem = &report.problems[0];
    assert_eq!(problem.contexts.len(), 1);
    assert_eq!(problem.contexts[0].machine, "test");
    assert!(problem.contexts[0].harness.is_none());
    assert!(problem.contexts[0].skill.is_none());
    assert_eq!(problem.path, "machines/test.toml");
    assert_eq!(problem.message, "Harness name must be a string: 42");
    assert_eq!(report.profiles[0].harnesses[0].harness, "codex");
    assert_eq!(report.profiles[0].harnesses[0].skills_rendered, 1);
    assert!(!f.settings.state_dir.exists());
}

use super::*;

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
fn copy_back_preview_labels_source_as_old_and_installed_copy_as_new() {
    use ratatoskr_core::{ConflictAction, preview_resolution, preview_source_edit};
    let f = Fixture::new();
    let original = "---\nname: review-code\ndescription: Review changed code\n---\nOriginal.\n";
    f.write("skills/review-code/SKILL.md", original);
    apply(&f.settings).unwrap();
    let path = f.output().join("SKILL.md");
    fs::write(&path, original.replace("Original.", "Hand edit.")).unwrap();
    let preview = serde_json::to_value(
        preview_resolution(&f.settings, &path, ConflictAction::CopyBack).unwrap(),
    )
    .unwrap();
    let diff = preview["effects"][0]["diff"].as_str().unwrap();
    assert!(
        diff.starts_with("--- source\n+++ installed copy\n"),
        "{diff}"
    );
    assert!(diff.contains("-Original.\n+Hand edit.\n"), "{diff}");

    let edited = serde_json::to_value(
        preview_source_edit(
            &f.settings,
            &path,
            &original.replace("Original.", "Draft edit."),
        )
        .unwrap(),
    )
    .unwrap();
    let diff = edited["effects"][0]["diff"].as_str().unwrap();
    assert!(
        diff.starts_with("--- source\n+++ edited source\n"),
        "{diff}"
    );
    assert!(diff.contains("-Original.\n+Draft edit.\n"), "{diff}");
}

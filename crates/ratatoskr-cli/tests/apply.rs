use std::{fs, process::Command};

fn output(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ratatoskr"));
    command.env_remove("RATATOSKR_CONFIG");
    command.arg("--config").arg(root.join("config.toml"));
    command.args(args);
    command.output().unwrap()
}

fn stdout(output: &std::process::Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn apply_reports_written_files_and_a_later_apply_is_unchanged() {
    let temporary = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temporary.path()).unwrap();
    let source = root.join("content");
    fs::create_dir_all(source.join("machines")).unwrap();
    fs::create_dir_all(source.join("skills/review-code")).unwrap();
    fs::write(source.join("harnesses.toml"), "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\n[codex.roots]\nmacos='{{af:home}}/.agents'\nwindows='{{af:home}}/.agents'\nlinux='{{af:home}}/.agents'\n").unwrap();
    fs::write(
        source.join("machines/test.toml"),
        format!(
            "os='{}'\nmode='staging'\nharnesses=['codex']\n",
            std::env::consts::OS
        ),
    )
    .unwrap();
    fs::write(source.join("skills/review-code/skill.toml"), "").unwrap();
    fs::write(
        source.join("skills/review-code/SKILL.md"),
        "---\nname: review-code\ndescription: Review code\n---\nRead the changes.\n",
    )
    .unwrap();
    stdout(&output(
        &root,
        &[
            "init",
            "--source",
            source.to_str().unwrap(),
            "--machine",
            "test",
        ],
    ));

    let added = stdout(&output(&root, &["apply"]));
    assert!(
        added.contains("Applied 1 change: 1 added, 0 updated, 0 removed."),
        "{added}"
    );
    assert!(added.contains("Generated files are up to date."), "{added}");

    fs::write(source.join("skills/review-code/extra.txt"), "first").unwrap();
    let updated_add = stdout(&output(&root, &["apply"]));
    assert!(
        updated_add.contains("Applied 1 change: 1 added, 0 updated, 0 removed."),
        "{updated_add}"
    );

    fs::write(source.join("skills/review-code/extra.txt"), "second").unwrap();
    let updated = stdout(&output(&root, &["apply"]));
    assert!(
        updated.contains("Applied 1 change: 0 added, 1 updated, 0 removed."),
        "{updated}"
    );

    fs::remove_file(source.join("skills/review-code/extra.txt")).unwrap();
    let removed = stdout(&output(&root, &["apply"]));
    assert!(
        removed.contains("Applied 1 change: 0 added, 0 updated, 1 removed."),
        "{removed}"
    );

    let unchanged = stdout(&output(&root, &["apply"]));
    assert!(!unchanged.contains("Applied "), "{unchanged}");
    assert!(
        unchanged.contains("Generated files are up to date."),
        "{unchanged}"
    );

    let json = stdout(&output(&root, &["--json", "apply"]));
    assert!(!json.contains("Applied "), "{json}");
    let report: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(report["changes"].as_array().unwrap().is_empty(), "{report}");
}

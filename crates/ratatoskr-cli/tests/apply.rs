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

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
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

    (temporary, root, source)
}

#[test]
fn apply_reports_written_files_and_a_later_apply_is_unchanged() {
    let (_temporary, root, source) = fixture();
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
    assert_eq!(
        report["applied"],
        serde_json::json!({"added": 0, "updated": 0, "removed": 0})
    );
    let planned: serde_json::Value =
        serde_json::from_str(&stdout(&output(&root, &["--json", "plan"]))).unwrap();
    assert!(planned.get("applied").is_none(), "{planned}");
}

#[test]
fn stale_target_cli_lists_previews_and_resolves_each_location() {
    for action in ["forget", "remove"] {
        let (_temporary, root, source) = fixture();
        let definitions = fs::read_to_string(source.join("harnesses.toml")).unwrap();
        fs::write(source.join("harnesses.toml"), format!("{definitions}[beta]\nskills_dir='skills'\nglobal_file='INSTRUCTIONS.md'\n[beta.roots]\nmacos='{{{{af:home}}}}/.beta'\nwindows='{{{{af:home}}}}/.beta'\nlinux='{{{{af:home}}}}/.beta'\n")).unwrap();
        let profile = format!("os='{}'\nmode='staging'\n", std::env::consts::OS);
        fs::write(
            source.join("machines/test.toml"),
            format!("{profile}harnesses=['codex','beta']\n"),
        )
        .unwrap();
        fs::create_dir_all(source.join("global")).unwrap();
        fs::write(source.join("global/AGENTS.md"), "Global instructions").unwrap();
        stdout(&output(&root, &["apply"]));
        fs::write(
            source.join("machines/test.toml"),
            format!("{profile}harnesses=['beta']\n"),
        )
        .unwrap();
        let planned = stdout(&output(&root, &["plan"]));
        assert!(
            planned.contains("Pending stale-target decision")
                && planned.contains("ratatoskr stale-target resolve"),
            "{planned}"
        );
        let status = output(&root, &["status"]);
        assert_eq!(status.status.code(), Some(1));
        let apply = output(&root, &["apply"]);
        assert_eq!(apply.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&apply.stderr).contains("Pending stale-target decision"));
        let locations: serde_json::Value =
            serde_json::from_str(&stdout(&output(&root, &["--json", "stale-target", "list"])))
                .unwrap();
        assert_eq!(locations.as_array().unwrap().len(), 2);
        let mut installed = vec![];
        for location in locations.as_array().unwrap() {
            for file in location["files"].as_array().unwrap() {
                let path = std::path::PathBuf::from(file["path"].as_str().unwrap());
                installed.push((path.clone(), fs::read(path).unwrap()));
            }
            let path = location["path"].as_str().unwrap();
            let preview = stdout(&output(
                &root,
                &["stale-target", "preview", path, "--action", action],
            ));
            let saved = root.join("preview.json");
            fs::write(&saved, &preview).unwrap();
            let result = stdout(&output(
                &root,
                &[
                    "stale-target",
                    "resolve",
                    "--preview",
                    saved.to_str().unwrap(),
                ],
            ));
            if action == "remove" {
                assert!(
                    result.contains("Nothing deleted") && result.contains("receipt.json"),
                    "{result}"
                );
            } else {
                assert!(result.contains("Installed files unchanged"), "{result}");
            }
        }
        for (path, bytes) in installed {
            if action == "forget" {
                assert_eq!(fs::read(path).unwrap(), bytes);
            } else {
                assert!(!path.exists());
            }
        }
        let report: serde_json::Value =
            serde_json::from_str(&stdout(&output(&root, &["--json", "plan"]))).unwrap();
        assert!(report["stale_targets"].as_array().unwrap().is_empty());
        assert!(report["changes"].as_array().unwrap().is_empty());
        assert!(report["conflicts"].as_array().unwrap().is_empty());
    }
}

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn source(root: &Path) {
    fs::create_dir_all(root.join("machines")).unwrap();
    fs::create_dir_all(root.join("skills/review-code")).unwrap();
    fs::write(root.join("harnesses.toml"), "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\n[codex.roots]\nlinux='{{af:home}}/.agents'\n").unwrap();
    fs::write(
        root.join("machines/remote.toml"),
        "os='linux'\nmode='live'\nhome='/remote/home'\nharnesses=['codex']\n",
    )
    .unwrap();
    fs::write(root.join("skills/review-code/skill.toml"), "").unwrap();
    fs::write(
        root.join("skills/review-code/SKILL.md"),
        "---\nname: review-code\ndescription: Review code\n---\nRead the changes.\n",
    )
    .unwrap();
}

fn validate(root: &Path, json: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ratatoskr"));
    // A malformed default-config override must never be resolved by this command.
    command.env("RATATOSKR_CONFIG", "relative-invalid-config.toml");
    command.arg("validate").arg(root);
    if json {
        command.arg("--json");
    }
    command.output().unwrap()
}

#[test]
fn validates_without_config_and_emits_human_and_json_reports() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    source(&root);
    let human = validate(&root, false);
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    assert!(
        String::from_utf8(human.stdout).unwrap().contains(
            "remote: 1 skill installation across 1 harness rendered; 0 problem occurrences"
        )
    );
    let json = validate(&root, true);
    assert!(json.status.success());
    let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(report["problems"], serde_json::json!([]));
    assert_eq!(report["profiles"][0]["harnesses"][0]["skills_rendered"], 1);
    assert!(!root.join(".validation").exists());
    assert!(!root.join("relative-invalid-config.toml").exists());
}

#[test]
fn returns_one_with_all_render_problems_and_two_for_unreadable_sources() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    source(&root);
    fs::write(
        root.join("skills/review-code/one.txt"),
        "{{af:missing_one}}",
    )
    .unwrap();
    fs::write(
        root.join("skills/review-code/two.txt"),
        "{{af:missing_two}}",
    )
    .unwrap();
    let json = validate(&root, true);
    assert_eq!(json.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(report["problems"].as_array().unwrap().len(), 2);
    for problem in report["problems"].as_array().unwrap() {
        assert_eq!(
            problem["contexts"],
            serde_json::json!([{
                "machine": "remote", "harness": "codex", "skill": "review-code"
            }])
        );
    }
    let human = validate(&root, false);
    assert_eq!(human.status.code(), Some(1));
    let output = String::from_utf8(human.stdout).unwrap();
    assert!(output.contains("missing_one") && output.contains("missing_two"));
    let missing = validate(&root.join("absent"), true);
    assert_eq!(missing.status.code(), Some(2));
    assert!(!missing.stderr.is_empty());
}

#[test]
fn groups_shared_problems_with_all_contexts_and_consistent_occurrence_counts() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    source(&root);
    fs::write(root.join("harnesses.toml"), "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\n[codex.roots]\nlinux='{{af:home}}/.agents'\n[claude]\nskills_dir='skills'\nglobal_file='CLAUDE.md'\n[claude.roots]\nlinux='{{af:home}}/.claude'\n").unwrap();
    for machine in ["local", "remote"] {
        fs::write(
            root.join(format!("machines/{machine}.toml")),
            "os='linux'\nmode='live'\nhome='/remote/home'\nharnesses=['codex', 'claude']\n",
        )
        .unwrap();
    }
    fs::write(
        root.join("skills/review-code/extra.txt"),
        "{{af:paths.missing}}",
    )
    .unwrap();
    // Global failures have harness contexts but no skill context.
    fs::create_dir(root.join("global")).unwrap();
    fs::write(root.join("global/AGENTS.md"), "{{af:paths.missing}}").unwrap();

    let json = validate(&root, true);
    assert_eq!(json.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(report["problem_occurrences"], 8);
    assert_eq!(report["problems"].as_array().unwrap().len(), 2);
    for profile in report["profiles"].as_array().unwrap() {
        assert_eq!(profile["problem_occurrences"], 4);
    }
    for problem in report["problems"].as_array().unwrap() {
        let skill = if problem["path"].as_str().unwrap().ends_with("extra.txt") {
            serde_json::json!("review-code")
        } else {
            serde_json::Value::Null
        };
        assert_eq!(
            problem["contexts"],
            serde_json::json!([
                {"machine": "local", "harness": "claude", "skill": skill},
                {"machine": "local", "harness": "codex", "skill": skill},
                {"machine": "remote", "harness": "claude", "skill": skill},
                {"machine": "remote", "harness": "codex", "skill": skill}
            ])
        );
    }
    let human = validate(&root, false);
    assert_eq!(human.status.code(), Some(1));
    let output = String::from_utf8(human.stdout).unwrap();
    for machine in ["local", "remote"] {
        assert!(output.contains(&format!("{machine}: 0 skill installations across 2 harnesses rendered; 4 problem occurrences")), "{output}");
    }
    assert!(output.contains("[local/claude/review-code, local/codex/review-code, remote/claude/review-code, remote/codex/review-code]"), "{output}");
    assert!(
        output.contains("[local/claude, local/codex, remote/claude, remote/codex]"),
        "{output}"
    );
    assert!(
        output.contains("2 profiles; 2 distinct problems; 8 problem occurrences"),
        "{output}"
    );
    assert_eq!(
        output
            .lines()
            .filter(|line| line.starts_with("ERROR "))
            .count(),
        2
    );
}

#[test]
fn undefined_path_reports_only_affected_machines_and_uses_singular_labels() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    source(&root);
    fs::write(root.join("machines/local.toml"), "os='linux'\nmode='live'\nhome='/local/home'\nharnesses=['codex']\n[paths]\nvault='/knowledge'\n").unwrap();
    fs::write(
        root.join("skills/review-code/extra.txt"),
        "{{af:paths.vault}}",
    )
    .unwrap();
    let json = validate(&root, true);
    assert_eq!(json.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(report["problem_occurrences"], 1);
    assert_eq!(report["profiles"][0]["machine"], "local");
    assert_eq!(report["profiles"][0]["problem_occurrences"], 0);
    assert_eq!(report["profiles"][1]["machine"], "remote");
    assert_eq!(report["profiles"][1]["problem_occurrences"], 1);
    assert_eq!(report["problems"].as_array().unwrap().len(), 1);
    assert_eq!(
        report["problems"][0]["contexts"],
        serde_json::json!([
            {"machine": "remote", "harness": "codex", "skill": "review-code"}
        ])
    );
    let human = validate(&root, false);
    let output = String::from_utf8(human.stdout).unwrap();
    assert_eq!(human.status.code(), Some(1));
    assert!(output.contains("1 problem occurrence\n"), "{output}");
    assert!(output.contains("ERROR skills/review-code/extra.txt: Undefined variable: paths.vault [remote/codex/review-code]"), "{output}");
    assert!(
        output.contains("2 profiles; 1 distinct problem; 1 problem occurrence"),
        "{output}"
    );
}

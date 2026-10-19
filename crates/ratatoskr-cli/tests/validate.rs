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
        String::from_utf8(human.stdout)
            .unwrap()
            .contains("remote: 1 skill installations across 1 harnesses rendered; 0 problems")
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
        assert_eq!(problem["machine"], "remote");
        assert_eq!(problem["harness"], "codex");
        assert_eq!(problem["skill"], "review-code");
    }
    let human = validate(&root, false);
    assert_eq!(human.status.code(), Some(1));
    let output = String::from_utf8(human.stdout).unwrap();
    assert!(output.contains("missing_one") && output.contains("missing_two"));
    let missing = validate(&root.join("absent"), true);
    assert_eq!(missing.status.code(), Some(2));
    assert!(!missing.stderr.is_empty());
}

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
    let stdout = String::from_utf8(human.stdout).unwrap();
    assert!(
        stdout.contains(
            "remote: 1 skill installation across 1 harness rendered; 0 problem occurrences"
        )
    );
    assert!(!stdout.contains("WARNING"), "{stdout}");
    let json = validate(&root, true);
    assert!(json.status.success());
    let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(report["problems"], serde_json::json!([]));
    assert_eq!(report["warnings"], serde_json::json!([]));
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
    assert_eq!(report["problems"][0]["path"], "global/AGENTS.md");
    assert_eq!(
        report["problems"][1]["path"],
        "skills/review-code/extra.txt"
    );
    for profile in report["profiles"].as_array().unwrap() {
        assert_eq!(profile["problem_occurrences"], 4);
    }
    for problem in report["problems"].as_array().unwrap() {
        let skill = if problem["path"] == "skills/review-code/extra.txt" {
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
    assert!(output.contains("ERROR global/AGENTS.md:"), "{output}");
    assert!(
        output.contains("ERROR skills/review-code/extra.txt:"),
        "{output}"
    );
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
        report["problems"][0]["path"],
        "skills/review-code/extra.txt"
    );
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

#[test]
fn reports_selector_values_that_can_never_match() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    source(&root);
    fs::write(
        root.join("skills/review-code/SKILL.md"),
        "\
---
name: review-code
description: Review code
---
Shared.
<!-- only harness=cdex -->
Typo.
<!-- /only -->
<!-- only os=mac -->
Mac.
<!-- /only -->
<!-- only harness=codex,cluade -->
Mixed.
<!-- /only -->
<!-- only harness=codex -->
Known.
<!-- /only -->
",
    )
    .unwrap();
    fs::write(
        root.join("skills/review-code/notes.md"),
        "<!-- only machine=brunnr -->\nNope.\n<!-- /only -->\n",
    )
    .unwrap();
    fs::write(
        root.join("skills/review-code/skill.toml"),
        "[target]\nmachines = ['remtoe']\nharnesses = ['cluade']\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("skills/review-code/variants/harness=codx")).unwrap();
    fs::write(
        root.join("skills/review-code/variants/harness=codx/SKILL.md"),
        "Variant.\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("skills/review-code/variants/harness=codex")).unwrap();
    fs::create_dir_all(root.join("global")).unwrap();
    fs::write(
        root.join("global/AGENTS.md"),
        "<!-- only harness=cursro -->\nTypo.\n<!-- /only -->\n",
    )
    .unwrap();
    let human = validate(&root, false);
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    let stdout = String::from_utf8(human.stdout).unwrap();
    for expected in [
        "WARNING skills/review-code/SKILL.md: Unknown harness `cdex` never matches; known: codex",
        "WARNING skills/review-code/SKILL.md: Unknown os `mac` never matches; known: linux, macos, windows",
        "WARNING skills/review-code/SKILL.md: Unknown harness `cluade` never matches; known: codex",
        "WARNING skills/review-code/notes.md: Unknown machine `brunnr` never matches; known: remote",
        "WARNING skills/review-code/skill.toml: Unknown machine `remtoe` never matches; known: remote",
        "WARNING skills/review-code/skill.toml: Unknown harness `cluade` never matches; known: codex",
        "WARNING skills/review-code/variants/harness=codx: Unknown harness `codx` never matches; known: codex",
        "WARNING global/AGENTS.md: Unknown harness `cursro` never matches; known: codex",
    ] {
        assert!(stdout.contains(expected), "missing {expected}\n{stdout}");
    }
    assert!(!stdout.contains("Unknown harness `codex`"), "{stdout}");
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with("WARNING "))
            .count(),
        8
    );
    let json = validate(&root, true);
    assert!(json.status.success());
    let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert!(
        report["problems"].as_array().unwrap().is_empty(),
        "{report}"
    );
    assert_eq!(report["warnings"].as_array().unwrap().len(), 8);
}

#[test]
fn selector_scan_failures_are_problems_and_missing_optional_paths_are_not() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("machines")).unwrap();
    fs::write(root.join("harnesses.toml"), "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\n[codex.roots]\nlinux='{{af:home}}/.agents'\n").unwrap();
    fs::write(
        root.join("machines/remote.toml"),
        "os='linux'\nmode='live'\nhome='/remote/home'\nharnesses=['codex']\n",
    )
    .unwrap();
    let clean = validate(&root, false);
    assert!(
        clean.status.success(),
        "{}",
        String::from_utf8_lossy(&clean.stderr)
    );
    let clean_out = String::from_utf8(clean.stdout).unwrap();
    assert!(!clean_out.contains("ERROR "), "{clean_out}");
    assert!(
        clean_out.contains("1 profile; 0 distinct problems; 0 problem occurrences"),
        "{clean_out}"
    );

    fs::create_dir_all(root.join("skills/parked")).unwrap();
    fs::write(root.join("skills/parked/skill.toml"), "enabled = false\n").unwrap();
    fs::write(
        root.join("skills/parked/broken.md"),
        "<!-- only os=mac -->\nOpen.\n",
    )
    .unwrap();
    fs::write(root.join("skills/parked/variants"), "not-a-directory").unwrap();
    let failed = validate(&root, true);
    assert_eq!(failed.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&failed.stdout).unwrap();
    let problems = report["problems"].as_array().unwrap();
    assert!(
        problems.iter().any(|problem| {
            problem["path"] == "skills/parked/broken.md"
                && problem["message"]
                    .as_str()
                    .unwrap()
                    .contains("Unclosed only block")
        }),
        "{report}"
    );
    assert!(
        problems.iter().any(|problem| {
            problem["path"] == "skills/parked/variants"
                && problem["message"]
                    .as_str()
                    .unwrap()
                    .starts_with("Could not inspect ")
        }),
        "{report}"
    );
    assert!(
        problems
            .iter()
            .all(|problem| problem["path"] != "global/AGENTS.md" && problem["path"] != "skills"),
        "{report}"
    );
}

use super::*;

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
    assert_eq!(report.problems.len(), 4, "{:?}", report.problems);
    assert_eq!(
        report
            .problems
            .iter()
            .map(|p| p.path.as_str())
            .collect::<Vec<_>>(),
        [
            "skills/malformed",
            "skills/malformed/skill.toml",
            "skills/missing",
            "skills/review-code"
        ]
    );
    let inspection = report
        .problems
        .iter()
        .find(|problem| problem.path == "skills/malformed/skill.toml")
        .unwrap();
    assert!(
        inspection
            .message
            .starts_with("Could not parse skills/malformed/skill.toml:"),
        "{}",
        inspection.message
    );
    assert!(
        inspection
            .contexts
            .iter()
            .all(|context| context.harness.is_none() && context.skill.is_none())
    );
    for problem in report
        .problems
        .iter()
        .filter(|problem| problem.path != "skills/malformed/skill.toml")
    {
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
            profile.problem_occurrences, 7,
            "Counts must retain every failing target and the selector-scan failure"
        );
        assert_eq!(profile.harnesses.len(), 2);
        assert!(profile.harnesses.iter().all(|h| h.skills_rendered == 0));
    }
    assert_eq!(report.problem_occurrences, 21);
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

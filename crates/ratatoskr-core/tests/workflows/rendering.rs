use super::*;

#[test]
fn selector_scan_distinguishes_missing_optional_paths_from_inspection_failures() {
    let f = Fixture::new();
    fs::remove_dir_all(f.settings.source.join("skills")).unwrap();
    let report = ratatoskr_core::validate_source(&f.settings.source).unwrap();
    assert!(
        report.problems.is_empty(),
        "missing skills/ and global/AGENTS.md are optional: {:?}",
        report.problems
    );
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);

    // `enabled = false` returns before the renderer walks files, so only the selector scan sees these.
    f.write("skills/parked/skill.toml", "enabled = false\n");
    f.write(
        "skills/parked/notes.md",
        "<!-- only harness=cursro -->\nHidden.\n<!-- /only -->\n",
    );
    f.write("skills/parked/broken.md", "<!-- only os=mac -->\nOpen.\n");
    fs::write(
        f.settings.source.join("skills/parked/variants"),
        "not-a-directory",
    )
    .unwrap();
    f.write("skills/garbled/skill.toml", "target = [");
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        "notes.md",
        f.settings.source.join("skills/parked/linked.md"),
    )
    .unwrap();

    let report = ratatoskr_core::validate_source(&f.settings.source).unwrap();
    let messages = report
        .problems
        .iter()
        .map(|problem| format!("{}: {}", problem.path, problem.message))
        .collect::<Vec<_>>();
    assert!(
        messages.iter().any(|message| {
            message.starts_with("skills/parked/broken.md: Could not parse ")
                && message.contains("Unclosed only block")
        }),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.starts_with("skills/parked/variants: Could not inspect ")),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.starts_with("skills/garbled/skill.toml: Could not parse ")),
        "{messages:?}"
    );
    #[cfg(unix)]
    assert!(
        messages.iter().any(|message| {
            message.starts_with("skills/parked/linked.md: Could not inspect ")
                && message.contains("symlink")
        }),
        "{messages:?}"
    );
    assert!(
        report.warnings.iter().any(|warning| {
            warning.path == "skills/parked/notes.md" && warning.message.contains("`cursro`")
        }),
        "{:?}",
        report.warnings
    );
    assert!(
        report
            .problems
            .iter()
            .all(|problem| problem.path != "global/AGENTS.md" && problem.path != "skills")
    );
    let variants = report
        .problems
        .iter()
        .find(|problem| problem.path == "skills/parked/variants")
        .unwrap();
    assert!(
        variants
            .contexts
            .iter()
            .all(|context| context.harness.is_none() && context.skill.is_none())
    );
    assert_eq!(variants.contexts.len(), report.profiles.len());
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
fn finder_metadata_in_skills_root_is_ignored() {
    let f = Fixture::new();
    f.write("skills/.DS_Store", "finder");
    f.write("skills/cache.pyc", "bytecode");
    let planned = plan(&f.settings).unwrap();
    assert_eq!(planned.changes.len(), 1);
    assert_eq!(ratatoskr_core::list_skills(&f.settings).unwrap().len(), 1);
}

#[test]
fn skills_root_file_error_names_the_entry() {
    let f = Fixture::new();
    f.write("skills/README.md", "notes");
    let error = format!("{:#}", plan(&f.settings).unwrap_err());
    assert!(
        error.contains("Skills root must contain directories only: README.md"),
        "{error}"
    );
    let Err(error) = ratatoskr_core::list_skills(&f.settings) else {
        panic!("README.md must be rejected");
    };
    let listed = format!("{error:#}");
    assert!(
        listed.contains("Skills root must contain directories only: README.md"),
        "{listed}"
    );
}

#[cfg(windows)]
#[test]
fn junction_skill_folder_is_refused_by_name() {
    let f = Fixture::new();
    let skills = f.settings.source.join("skills");
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(skills.join("linked"))
        .arg(skills.join("review-code"))
        .status()
        .unwrap();
    assert!(status.success(), "mklink /J failed: {status}");
    let error = format!("{:#}", plan(&f.settings).unwrap_err());
    assert!(
        error.contains("Skills root entry is a symlink: linked"),
        "{error}"
    );
    let Err(error) = ratatoskr_core::list_skills(&f.settings) else {
        panic!("junction skill folder must be rejected");
    };
    let listed = format!("{error:#}");
    assert!(
        listed.contains("Skills root entry is a symlink: linked"),
        "{listed}"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_skill_folder_is_refused_by_name() {
    let f = Fixture::new();
    std::os::unix::fs::symlink("review-code", f.settings.source.join("skills/linked")).unwrap();
    let error = format!("{:#}", plan(&f.settings).unwrap_err());
    assert!(
        error.contains("Skills root entry is a symlink: linked"),
        "{error}"
    );
    let Err(error) = ratatoskr_core::list_skills(&f.settings) else {
        panic!("symlinked skill folder must be rejected");
    };
    let listed = format!("{error:#}");
    assert!(
        listed.contains("Skills root entry is a symlink: linked"),
        "{listed}"
    );
}

#[test]
fn invalid_identifier_names_the_accepted_form() {
    let f = Fixture::new();
    fs::create_dir(f.settings.source.join("skills/Bad_Name")).unwrap();
    let error = format!("{:#}", plan(&f.settings).unwrap_err());
    assert!(
        error.contains("Invalid identifier: \"Bad_Name\""),
        "{error}"
    );
    assert!(error.contains("lowercase ASCII letters"), "{error}");
    assert!(error.contains("`review-code`"), "{error}");
}

#[test]
fn variants_ignore_runtime_artifacts_but_name_real_stray_files() {
    let f = Fixture::new();
    for file in [".DS_Store", "cache.pyc", "__pycache__/cache.pyc"] {
        f.write(&format!("skills/review-code/variants/{file}"), "runtime");
    }
    f.write(
        "skills/review-code/variants/harness=codex/guide.txt",
        "Guide",
    );
    assert_eq!(plan(&f.settings).unwrap().changes.len(), 2);
    let report = ratatoskr_core::validate_source(&f.settings.source).unwrap();
    assert!(
        report.problems.is_empty(),
        "{}",
        serde_json::to_string(&report).unwrap()
    );
    f.write("skills/review-code/variants/notes.txt", "stray");
    let error = format!("{:#}", plan(&f.settings).unwrap_err());
    assert!(
        error.contains("Variants must be directories: notes.txt"),
        "{error}"
    );
    let report = ratatoskr_core::validate_source(&f.settings.source).unwrap();
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.path.ends_with("variants/notes.txt"))
    );
}

#[cfg(unix)]
#[test]
fn variants_refuse_runtime_named_symlinks() {
    let f = Fixture::new();
    f.write(
        "skills/review-code/variants/harness=codex/guide.txt",
        "Guide",
    );
    std::os::unix::fs::symlink(
        f.settings.source.join("skills/review-code/SKILL.md"),
        f.settings
            .source
            .join("skills/review-code/variants/.DS_Store"),
    )
    .unwrap();
    let error = format!("{:#}", plan(&f.settings).unwrap_err());
    assert!(
        error.contains("symlink") && error.contains(".DS_Store"),
        "{error}"
    );
    assert!(
        !ratatoskr_core::validate_source(&f.settings.source)
            .unwrap()
            .problems
            .is_empty()
    );
}

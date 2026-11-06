use super::*;

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
    apply(&f.settings).unwrap();
    let archived = ratatoskr_core::archive_skill(&f.settings, None, "review-code").unwrap();
    assert!(archived.join("SKILL.md").is_file());
    assert!(!f.settings.source.join("skills/review-code").exists());
    // Archiving keeps the source but retires the installed copy on the next apply.
    let retired = plan(&f.settings).unwrap();
    assert_eq!(retired.skills, 0);
    assert!(
        retired
            .changes
            .iter()
            .any(|c| c.action == "remove" && c.path == f.output().join("SKILL.md"))
    );
    ratatoskr_core::create_skill(&f.settings, None, "review-code", &original.markdown).unwrap();
    assert_eq!(plan(&f.settings).unwrap().skills, 1);
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
fn structured_save_checks_remote_profiles_for_global_and_project_skills() {
    for project_scoped in [false, true] {
        let f = Fixture::new();
        let harnesses = fs::read_to_string(f.settings.source.join("harnesses.toml")).unwrap();
        f.write(
            "harnesses.toml",
            &format!("{harnesses}{}", harnesses.replace("codex", "claude")),
        );
        // The foreign live profile must be rendered without touching its home.
        f.write(
            "machines/other.toml",
            "os='windows'\nmode='live'\nhome='C:/Users/remote'\nharnesses=['claude']\n",
        );
        let mut settings = f.settings.clone();
        let project = if project_scoped {
            let root = f.root.join("project");
            fs::create_dir_all(&root).unwrap();
            git(&root, &["init", "-b", "main"]);
            fs::write(root.join(".ratatoskr.toml"), "source='agent-skills'\n").unwrap();
            settings.projects.push(root.clone());
            Some(root)
        } else {
            None
        };
        let selected = project
            .as_ref()
            .map(|root| root.join("agent-skills"))
            .unwrap_or_else(|| settings.source.clone());
        let skill = selected.join("skills/review-code");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("skill.toml"), "").unwrap();
        let initial = "---\nname: review-code\ndescription: Review changes\n---\nReview changes.\n";
        fs::write(skill.join("SKILL.md"), initial).unwrap();
        // An unrelated, already broken remote-only skill must not block saves.
        let unrelated = selected.join("skills/unrelated");
        fs::create_dir_all(&unrelated).unwrap();
        fs::write(
            unrelated.join("skill.toml"),
            "[target]\nharnesses=['claude']\n",
        )
        .unwrap();
        fs::write(
            unrelated.join("SKILL.md"),
            "---\nname: unrelated\ndescription: ''\n---\nBody\n",
        )
        .unwrap();
        let markdown = |remote: usize| {
            format!(
                "---\nname: review-code\ndescription: \"{}<!-- only harness=codex --> {}<!-- /only --><!-- only harness=claude --> {}<!-- /only -->\"\n---\nReview changes.\n",
                "s".repeat(600),
                "c".repeat(400),
                "a".repeat(remote)
            )
        };
        let save_draft = |markdown: &str| {
            let current =
                ratatoskr_core::read_skill(&settings, project.as_deref(), "review-code").unwrap();
            ratatoskr_core::save_skill_draft(
                &settings,
                project.as_deref(),
                "review-code",
                markdown,
                &current.metadata,
                &current.markdown_hash,
                &current.metadata_hash,
            )
        };
        let overflow = markdown(424);
        let empty_remote_body = markdown(400).replace(
            "Review changes.",
            "<!-- only harness=codex -->Review changes.<!-- /only -->",
        );
        for (invalid, message) in [
            (&overflow, "Invalid skill description"),
            (&empty_remote_body, "Skill body is empty"),
        ] {
            let error = format!("{:#}", save_draft(invalid).unwrap_err());
            assert!(error.contains("other/claude"), "{error}");
            assert!(error.contains(message), "{error}");
            assert_eq!(fs::read_to_string(skill.join("SKILL.md")).unwrap(), initial);
            assert_eq!(fs::read_to_string(skill.join("skill.toml")).unwrap(), "");
        }
        let valid = markdown(400);
        save_draft(&valid).unwrap();
        assert_eq!(fs::read_to_string(skill.join("SKILL.md")).unwrap(), valid);
        // A pre-existing error in this skill on the other machine can be left
        // unchanged during a partial repair, and then fixed in a later save.
        fs::write(skill.join("SKILL.md"), &overflow).unwrap();
        let partial_repair = overflow.replace("Review changes.", "Review carefully.");
        save_draft(&partial_repair).unwrap();
        assert_eq!(
            fs::read_to_string(skill.join("SKILL.md")).unwrap(),
            partial_repair
        );
        save_draft(&valid).unwrap();
        assert_eq!(fs::read_to_string(skill.join("SKILL.md")).unwrap(), valid);
        // Skill-level diagnostics must also be checked when targeting changes
        // enable another machine with conflicting variants.
        let restricted = "[target]\nharnesses=['codex']\n";
        fs::write(skill.join("skill.toml"), restricted).unwrap();
        for variant in ["harness=claude", "machine=other"] {
            let path = skill.join("variants").join(variant);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("notes.txt"), "Conflicting variant\n").unwrap();
        }
        let current =
            ratatoskr_core::read_skill(&settings, project.as_deref(), "review-code").unwrap();
        let error = ratatoskr_core::save_skill_draft(
            &settings,
            project.as_deref(),
            "review-code",
            &valid,
            "",
            &current.markdown_hash,
            &current.metadata_hash,
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("skills/review-code (other/claude)"));
        assert_eq!(fs::read_to_string(skill.join("SKILL.md")).unwrap(), valid);
        assert_eq!(
            fs::read_to_string(skill.join("skill.toml")).unwrap(),
            restricted
        );
    }
}

#[test]
fn structured_save_limits_each_rendered_description_instead_of_the_template() {
    let f = Fixture::new();
    let harnesses = fs::read_to_string(f.settings.source.join("harnesses.toml")).unwrap();
    f.write(
        "harnesses.toml",
        &format!("{harnesses}{}", harnesses.replace("codex", "claude")),
    );
    let profile = fs::read_to_string(f.settings.source.join("machines/test.toml")).unwrap();
    f.write(
        "machines/test.toml",
        &profile.replace("harnesses=['codex']", "harnesses=['codex','claude']"),
    );
    let markdown = |codex: usize, claude: usize| {
        format!(
            "---\nname: review-code\ndescription: \"{}<!-- only harness=codex --> {}<!-- /only --><!-- only harness=claude --> {}<!-- /only -->\"\n---\nReview the changes.\n",
            "s".repeat(600),
            "c".repeat(codex),
            "a".repeat(claude)
        )
    };
    let valid = markdown(400, 400);
    assert!(
        ratatoskr_core::editor_view(&valid, "")
            .unwrap()
            .description
            .chars()
            .count()
            > 1024
    );
    for harness in ["codex", "claude"] {
        let files = ratatoskr_core::preview_skill(
            &f.settings,
            None,
            "review-code",
            &valid,
            "",
            "test",
            harness,
        )
        .unwrap();
        let rendered = files
            .iter()
            .find(|file| file.path == Path::new("SKILL.md"))
            .unwrap()
            .text
            .as_ref()
            .unwrap();
        assert_eq!(
            ratatoskr_core::editor_view(rendered, "")
                .unwrap()
                .description
                .chars()
                .count(),
            1001
        );
    }
    let before = ratatoskr_core::read_skill(&f.settings, None, "review-code").unwrap();
    ratatoskr_core::save_skill_draft(
        &f.settings,
        None,
        "review-code",
        &valid,
        &before.metadata,
        &before.markdown_hash,
        &before.metadata_hash,
    )
    .unwrap();
    let saved = ratatoskr_core::read_skill(&f.settings, None, "review-code").unwrap();
    assert_eq!(saved.markdown, valid);
    // Reject an overlong result for either agent, including one other than the
    // first preview target, without changing either source file.
    for invalid in [markdown(424, 400), markdown(400, 424)] {
        let error = ratatoskr_core::save_skill_draft(
            &f.settings,
            None,
            "review-code",
            &invalid,
            &saved.metadata,
            &saved.markdown_hash,
            &saved.metadata_hash,
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("Invalid skill description"));
        let unchanged = ratatoskr_core::read_skill(&f.settings, None, "review-code").unwrap();
        assert_eq!(unchanged.markdown, saved.markdown);
        assert_eq!(unchanged.metadata, saved.metadata);
    }
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
fn disabling_a_skill_cannot_hide_new_malformed_targeting() {
    let f = Fixture::new();
    let current = ratatoskr_core::read_skill(&f.settings, None, "review-code").unwrap();
    let error = ratatoskr_core::save_skill_draft(
        &f.settings,
        None,
        "review-code",
        &current.markdown,
        "enabled=false\n[target]\nharnesses='codex'\n",
        &current.markdown_hash,
        &current.metadata_hash,
    )
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("is not an array"),
        "{error:#}"
    );
    assert_eq!(
        ratatoskr_core::read_skill(&f.settings, None, "review-code")
            .unwrap()
            .metadata,
        current.metadata
    );
}

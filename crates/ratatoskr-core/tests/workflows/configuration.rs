use super::*;

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
fn global_instructions_save_validates_every_profile_and_previews_each_target() {
    use ratatoskr_core::{
        analyze_global_instructions, preview_global_instructions, preview_global_instructions_path,
        read_global_instructions, save_global_instructions,
    };
    let f = Fixture::new();
    f.write("harnesses.toml", "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\nglobal_root='{{af:home}}/.codex-home'\n[codex.roots]\nmacos='{{af:home}}/.codex'\nwindows='{{af:home}}/.codex'\nlinux='{{af:home}}/.codex'\n");
    f.write("machines/test.toml", "os='macos'\nmode='staging'\nharnesses=['codex']\ncapabilities=[]\n[paths]\nvault='/knowledge'\nnotes='/notes'\n");
    f.write("machines/remote.toml", "os='windows'\nmode='live'\nhome='Z:\\remote-user'\nharnesses=['codex']\n[paths]\nvault='Z:\\knowledge'\n");
    let global = f.settings.source.join("global/AGENTS.md");
    let empty = read_global_instructions(&f.settings).unwrap();
    assert_eq!(empty.text, "");

    let blank = save_global_instructions(&f.settings, " \n", &empty.hash).unwrap_err();
    assert!(format!("{blank:#}").contains("Global instructions are empty"));
    // The current machine renders this value; another profile lacks it.
    let missing = "Notes live in {{af:paths.notes}}.\n";
    let error = save_global_instructions(&f.settings, missing, &empty.hash).unwrap_err();
    assert!(
        format!("{error:#}").contains("global/AGENTS.md (remote/codex)"),
        "{error:#}"
    );
    assert!(!global.exists());

    let text = "Vault: {{af:paths.vault}}.\n<!-- only machine=test -->\nNotes live in {{af:paths.notes}}.\n<!-- /only -->\n";
    save_global_instructions(&f.settings, text, &empty.hash).unwrap();
    assert_eq!(fs::read_to_string(&global).unwrap(), text);
    assert!(save_global_instructions(&f.settings, "Other.\n", &empty.hash).is_err());
    assert_eq!(fs::read_to_string(&global).unwrap(), text);
    let saved = read_global_instructions(&f.settings).unwrap();
    assert!(
        ratatoskr_core::write_source_file(&f.settings, "global/AGENTS.md", "Other.\n", &saved.hash)
            .is_err()
    );

    let local = preview_global_instructions(&f.settings, text, "test", "codex").unwrap();
    assert!(local.contains("Vault: /knowledge.") && local.contains("Notes live in /notes."));
    let remote = preview_global_instructions(&f.settings, text, "remote", "codex").unwrap();
    assert!(remote.contains("Vault: Z:\\knowledge.") && !remote.contains("Notes"));
    assert!(
        analyze_global_instructions(&f.settings, text, "remote", "codex")
            .unwrap()
            .diagnostics
            .is_empty()
    );
    let analysis = analyze_global_instructions(&f.settings, missing, "remote", "codex").unwrap();
    assert!(analysis.diagnostics.iter().any(|d| d.severity == "error"
        && d.message.contains("Undefined variable: paths.notes")));

    assert_eq!(
        preview_global_instructions_path(&f.settings, "test", "codex").unwrap(),
        f.settings.state_dir.join("test/staging/codex/AGENTS.md")
    );
    assert_eq!(
        preview_global_instructions_path(&f.settings, "remote", "codex").unwrap(),
        PathBuf::from("Z:\\remote-user/.codex-home").join("AGENTS.md")
    );
    let staged = plan(&f.settings).unwrap();
    assert!(staged.changes.iter().any(|change| {
        change.path == f.settings.state_dir.join("test/staging/codex/AGENTS.md")
            && change
                .diff
                .as_deref()
                .unwrap_or("")
                .contains("Notes live in /notes.")
    }));
}

use super::*;

#[test]
fn project_monitor_counts_only_skill_changes_and_surfaces_missing_upstream() {
    let f = Fixture::new();
    let project = f.root.join("project");
    fs::create_dir_all(project.join("agent-skills")).unwrap();
    git(&project, &["init", "-b", "main"]);
    fs::write(project.join("agent-skills/SKILL.md"), "Skill source").unwrap();
    fs::write(project.join("unrelated.txt"), "Not a skill").unwrap();
    let config = f.root.join("config.toml");
    save(&config, &f.settings).unwrap();
    let status = add_project(&config, &project, "agent-skills").unwrap();
    assert_eq!(status.changed_files, 2); // Source plus project marker, not unrelated.txt.
    assert!(status.upstream.is_none());
    assert!(status.outgoing_commits.is_none());
    assert_eq!(load(&config).unwrap().projects, vec![project.clone()]);
    fs::remove_file(project.join(".ratatoskr.toml")).unwrap();
    assert!(project_status(&project).error.is_some());
}

#[test]
fn project_monitor_detects_only_unpushed_skill_commits() {
    let f = Fixture::new();
    let remote = f.root.join("remote");
    fs::create_dir(&remote).unwrap();
    git(&remote, &["init", "--bare", "-b", "main"]);
    let project = f.root.join("project");
    fs::create_dir_all(project.join("agent-skills")).unwrap();
    git(&project, &["init", "-b", "main"]);
    git(&project, &["config", "user.name", "Ratatoskr tests"]);
    git(&project, &["config", "user.email", "tests@localhost"]);
    git(&project, &["config", "commit.gpgsign", "false"]);
    fs::write(project.join("agent-skills/SKILL.md"), "Initial").unwrap();
    fs::write(project.join(".ratatoskr.toml"), "source='agent-skills'\n").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-m", "Initial"]);
    let remote_url = remote
        .to_str()
        .unwrap()
        .trim_start_matches("\\\\?\\")
        .replace('\\', "/");
    git(&project, &["remote", "add", "origin", &remote_url]);
    git(&project, &["push", "-u", "origin", "main"]);
    fs::write(project.join("unrelated.txt"), "Change").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-m", "Unrelated"]);
    assert_eq!(project_status(&project).outgoing_commits, Some(0));
    fs::write(project.join("agent-skills/SKILL.md"), "Changed").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-m", "Skills"]);
    assert_eq!(project_status(&project).outgoing_commits, Some(1));
}

#[test]
fn project_source_lookup_accepts_native_and_simplified_canonical_paths() {
    let f = Fixture::new();
    let project = f.root.join("project");
    fs::create_dir_all(project.join("agent-skills/skills")).unwrap();
    git(&project, &["init", "-b", "main"]);
    let config = f.root.join("config.toml");
    save(&config, &f.settings).unwrap();
    add_project(&config, &project, "agent-skills").unwrap();
    let settings = load(&config).unwrap();
    let native = project.canonicalize().unwrap();
    let simplified = dunce::canonicalize(&project).unwrap();
    assert_eq!(settings.projects, vec![native.clone()]);
    let markdown = ratatoskr_core::new_skill_markdown("project-review", "Review project").unwrap();
    ratatoskr_core::create_skill(&settings, Some(&simplified), "project-review", &markdown)
        .unwrap();
    for root in [&native, &simplified] {
        let skill = ratatoskr_core::read_skill(&settings, Some(root), "project-review").unwrap();
        assert_eq!(skill.markdown, markdown);
    }
    let unregistered = f.root.join("unregistered");
    fs::create_dir(&unregistered).unwrap();
    let error = ratatoskr_core::read_skill(&settings, Some(&unregistered), "project-review")
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("Project is not registered"),
        "{error:#}"
    );
}

#[cfg(windows)]
#[test]
fn windows_project_lookup_and_removal_accept_equivalent_path_spellings() {
    let f = Fixture::new();
    let project = f.root.join("project");
    fs::create_dir_all(project.join("agent-skills/skills")).unwrap();
    git(&project, &["init", "-b", "main"]);
    let config = f.root.join("config.toml");
    save(&config, &f.settings).unwrap();
    add_project(&config, &project, "agent-skills").unwrap();
    let settings = load(&config).unwrap();
    let native = project.canonicalize().unwrap();
    assert!(native.to_str().unwrap().starts_with("\\\\?\\"));
    assert_eq!(settings.projects, vec![native.clone()]);
    let markdown = ratatoskr_core::new_skill_markdown("project-review", "Review project").unwrap();
    ratatoskr_core::create_skill(&settings, Some(&native), "project-review", &markdown).unwrap();
    let plain = dunce::canonicalize(&project).unwrap();
    let plain = plain.to_str().unwrap();
    let flipped_drive = {
        let (drive, rest) = plain.split_at(1);
        let drive = if drive == drive.to_uppercase() {
            drive.to_lowercase()
        } else {
            drive.to_uppercase()
        };
        format!("{drive}{rest}")
    };
    let spellings = [
        native.to_str().unwrap().to_owned(),
        plain.to_owned(),
        format!("{plain}\\"),
        flipped_drive.clone(),
        plain.replace('\\', "/"),
        format!("{}/", flipped_drive.replace('\\', "/")),
    ];
    for spelling in &spellings {
        let root = PathBuf::from(spelling);
        let skill = ratatoskr_core::read_skill(&settings, Some(&root), "project-review")
            .unwrap_or_else(|error| panic!("{spelling}: {error:#}"));
        assert_eq!(skill.markdown, markdown);
        ratatoskr_core::preview_skill(
            &settings,
            Some(&root),
            "project-review",
            &skill.markdown,
            &skill.metadata,
            "test",
            "codex",
        )
        .unwrap_or_else(|error| panic!("{spelling}: {error:#}"));
    }
    apply(&settings).unwrap();
    assert_eq!(owned_project_roots(&settings), 1);
    let removal = PathBuf::from(spellings.last().unwrap());
    ratatoskr_core::remove_project(&config, &removal).unwrap();
    assert!(load(&config).unwrap().projects.is_empty());
    assert_eq!(owned_project_roots(&settings), 0);
}

fn owned_project_roots(settings: &Settings) -> usize {
    let manifest = settings
        .state_dir
        .join(&settings.machine)
        .join("manifest.json");
    let value: serde_json::Value = serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
    value["project_roots"].as_object().unwrap().len()
}

#[test]
fn project_removal_refuses_unregistered_roots_and_accepts_deleted_registered_roots() {
    let f = Fixture::new();
    let project = f.root.join("project");
    fs::create_dir_all(project.join("agent-skills/skills")).unwrap();
    git(&project, &["init", "-b", "main"]);
    let config = f.root.join("config.toml");
    save(&config, &f.settings).unwrap();
    add_project(&config, &project, "agent-skills").unwrap();
    let settings = load(&config).unwrap();
    let registered = settings.projects.clone();
    let markdown = ratatoskr_core::new_skill_markdown("project-review", "Review project").unwrap();
    ratatoskr_core::create_skill(&settings, Some(&project), "project-review", &markdown).unwrap();
    apply(&settings).unwrap();
    assert_eq!(owned_project_roots(&settings), 1);
    let unregistered = f.root.join("unregistered");
    fs::create_dir(&unregistered).unwrap();
    let error = ratatoskr_core::remove_project(&config, &unregistered)
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("Project is not registered"),
        "{error:#}"
    );
    assert_eq!(load(&config).unwrap().projects, registered);
    fs::remove_dir_all(&project).unwrap();
    ratatoskr_core::remove_project(&config, &registered[0]).unwrap();
    assert!(load(&config).unwrap().projects.is_empty());
    assert_eq!(owned_project_roots(&settings), 0);
}

#[test]
fn project_content_renders_with_shared_machine_settings_and_can_be_detached() {
    let f = Fixture::new();
    let project = f.root.join("project");
    fs::create_dir_all(project.join("agent-skills/skills/project-review")).unwrap();
    git(&project, &["init", "-b", "main"]);
    fs::write(
        project.join("agent-skills/skills/project-review/skill.toml"),
        "",
    )
    .unwrap();
    fs::write(
        project.join("agent-skills/skills/project-review/SKILL.md"),
        "---\nname: project-review\ndescription: Review this project\n---\nUse {{af:machine}}.\n",
    )
    .unwrap();
    let config = f.root.join("config.toml");
    save(&config, &f.settings).unwrap();
    add_project(&config, &project, "agent-skills").unwrap();
    let settings = load(&config).unwrap();
    let initial = plan(&settings).unwrap();
    assert_eq!(initial.skills, 2);
    let output = initial
        .changes
        .iter()
        .find(|c| c.path.ends_with("project-review/SKILL.md"))
        .unwrap()
        .path
        .clone();
    apply(&settings).unwrap();
    assert!(fs::read_to_string(&output).unwrap().contains("Use test."));
    let skills = output.parent().unwrap().parent().unwrap();
    assert_eq!(
        f.ownership()["target_harnesses"][skills.to_str().unwrap()],
        "codex"
    );
    ratatoskr_core::remove_project(&config, &project).unwrap();
    assert!(output.is_file(), "Detaching preserves project output");
    assert!(
        f.ownership()["target_harnesses"]
            .get(skills.to_str().unwrap())
            .is_none()
    );
    assert!(
        f.ownership()["target_harnesses"]
            .as_object()
            .unwrap()
            .keys()
            .all(|path| !Path::new(path).starts_with(skills.parent().unwrap()))
    );
    assert!(plan(&load(&config).unwrap()).unwrap().conflicts.is_empty());
}

#[test]
fn a_directory_whose_git_points_at_another_worktree_is_not_a_checkout_root() {
    let f = Fixture::new();
    let elsewhere = f.root.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let repository = f.root.join("repository");
    fs::create_dir_all(&repository).unwrap();
    git(&repository, &["init", "-b", "main"]);
    git(
        &repository,
        &["config", "core.worktree", &git_path(&elsewhere)],
    );
    let project = f.root.join("project");
    fs::create_dir_all(project.join("agent-skills/skills")).unwrap();
    fs::write(project.join(".ratatoskr.toml"), "source = 'agent-skills'\n").unwrap();
    fs::write(
        project.join(".git"),
        format!("gitdir: {}\n", git_path(&repository.join(".git"))),
    )
    .unwrap();
    let error = project_status(&project).error.unwrap();
    assert!(
        error.contains("Register the Git repository root"),
        "{error}"
    );
}

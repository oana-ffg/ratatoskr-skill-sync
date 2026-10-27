use ratatoskr_core::{self as core, DesktopSourceEdit as Edit, Settings};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    settings: Settings,
    config: PathBuf,
}
fn output(root: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap()
}
fn git(root: &Path, args: &[&str]) -> String {
    let result = output(root, args);
    assert!(
        result.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap().trim().into()
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let source = root.join("content");
        fs::create_dir_all(source.join("machines")).unwrap();
        fs::create_dir_all(source.join("skills/review")).unwrap();
        fs::create_dir_all(source.join("global")).unwrap();
        fs::write(source.join("harnesses.toml"), "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\nproject_root='.agents'\n[codex.roots]\nmacos='{{af:home}}/.agents'\nwindows='{{af:home}}/.agents'\nlinux='{{af:home}}/.agents'\n").unwrap();
        fs::write(
            source.join("machines/test.toml"),
            "os='macos'\nmode='staging'\nharnesses=['codex']\n",
        )
        .unwrap();
        fs::write(
            source.join("global/AGENTS.md"),
            "Original global instructions\n",
        )
        .unwrap();
        fs::write(
            source.join("skills/review/SKILL.md"),
            "---\nname: review\ndescription: Review code\n---\nReview the changes.\n",
        )
        .unwrap();
        fs::write(source.join("skills/review/skill.toml"), "").unwrap();
        fs::write(source.join("notes.txt"), "Original notes\n").unwrap();
        git(&source, &["init", "-b", "main"]);
        Self::identity(&source);
        git(&source, &["add", "."]);
        git(&source, &["commit", "-m", "Initial content"]);
        let remote = root.join("remote.git");
        fs::create_dir(&remote).unwrap();
        git(&remote, &["init", "--bare", "-b", "main"]);
        git(&source, &["remote", "add", "origin", "../remote.git"]);
        git(&source, &["push", "-u", "origin", "main"]);
        let mut settings = Settings {
            source,
            machine: "test".into(),
            state_dir: root.join("state"),
            ..Default::default()
        };
        settings.updates.automatic = false;
        let config = root.join("config.toml");
        core::save(&config, &settings).unwrap();
        Self {
            _temp: temp,
            root,
            settings,
            config,
        }
    }
    fn identity(root: &Path) {
        for (key, value) in [
            ("user.name", "Editor user"),
            ("user.email", "editor@localhost"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
            ("core.hooksPath", ".git/hooks"),
        ] {
            git(root, &["config", key, value]);
        }
    }
    fn edit(&self, edit: Edit<'_>) -> Option<PathBuf> {
        let result = core::desktop_source_edit(&self.settings, None, edit).unwrap();
        assert_eq!(self.head(), git(&self.remote(), &["rev-parse", "main"]));
        result
    }
    fn head(&self) -> String {
        git(&self.settings.source, &["rev-parse", "HEAD"])
    }
    fn remote(&self) -> PathBuf {
        self.root.join("remote.git")
    }
    fn remote_file(&self, file: &str) -> String {
        git(&self.remote(), &["show", &format!("main:{file}")])
    }
    fn save_global(&self, text: &str) -> anyhow::Result<Option<PathBuf>> {
        let current = core::read_global_instructions(&self.settings).unwrap();
        core::desktop_source_edit(
            &self.settings,
            None,
            Edit::GlobalInstructions {
                markdown: text,
                expected_hash: &current.hash,
            },
        )
    }
    fn cli(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ratatoskr"))
            .arg("--config")
            .arg(&self.config)
            .args(args)
            .output()
            .unwrap()
    }
    fn clone_peer(&self) -> PathBuf {
        // Set checkout policy before clone writes files, not just afterwards in identity().
        git(
            &self.root,
            &[
                "clone",
                "--config",
                "core.autocrlf=false",
                "remote.git",
                "peer",
            ],
        );
        let peer = self.root.join("peer");
        Self::identity(&peer);
        peer
    }
    fn hook(&self, name: &str, script: &str) {
        let path = self.settings.source.join(".git/hooks").join(name);
        fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
}

#[test]
fn desktop_commits_pushes_with_user_identity_and_explicit_deploy_refspec() {
    let f = Fixture::new();
    git(&f.settings.source, &["config", "push.default", "nothing"]);
    git(
        &f.settings.source,
        &["config", "remote.origin.push", "HEAD:refs/heads/wrong"],
    );
    f.save_global("Saved global instructions\n").unwrap();
    assert_eq!(f.head(), git(&f.remote(), &["rev-parse", "main"]));
    assert_eq!(
        f.remote_file("global/AGENTS.md"),
        "Saved global instructions"
    );
    assert_eq!(
        git(&f.settings.source, &["log", "-1", "--format=%an <%ae>|%s"]),
        "Editor user <editor@localhost>|Ratatoskr editor: update global instructions"
    );
    assert!(
        !output(&f.remote(), &["rev-parse", "--verify", "refs/heads/wrong"])
            .status
            .success()
    );
    assert!(!core::content_status(&f.settings).needs_attention());
    let head = f.head();
    f.save_global("Saved global instructions\n").unwrap();
    assert_eq!(
        f.head(),
        head,
        "Identical save must not create an empty commit"
    );
}

#[test]
fn desktop_commits_only_actually_changed_files_and_preserves_unrelated_index() {
    let f = Fixture::new();
    fs::write(
        f.settings.source.join("notes.txt"),
        "Unrelated staged change",
    )
    .unwrap();
    git(&f.settings.source, &["add", "notes.txt"]);
    fs::write(f.settings.source.join("untracked.txt"), "Unrelated draft").unwrap();
    fs::write(
        f.settings.source.join("skills/review/skill.toml"),
        "# Pre-existing metadata draft\n",
    )
    .unwrap();
    let current = core::read_skill(&f.settings, None, "review").unwrap();
    let markdown = current
        .markdown
        .replace("Review the changes.", "Review every change.");
    f.edit(Edit::SkillDraft {
        name: "review",
        markdown: &markdown,
        metadata: &current.metadata,
        markdown_hash: &current.markdown_hash,
        metadata_hash: &current.metadata_hash,
    });
    assert_eq!(
        git(
            &f.settings.source,
            &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"]
        ),
        "skills/review/SKILL.md"
    );
    assert_eq!(f.remote_file("notes.txt"), "Original notes");
    assert_eq!(f.remote_file("skills/review/skill.toml"), "");
    assert_eq!(
        git(&f.settings.source, &["diff", "--cached", "--name-only"]),
        "notes.txt"
    );
    assert_eq!(core::content_status(&f.settings).changed_files, 3);
    assert_eq!(core::content_status(&f.settings).outgoing_commits, Some(0));
    // A no-op save must not publish the pre-existing dirty metadata either.
    let head = f.head();
    let current = core::read_skill(&f.settings, None, "review").unwrap();
    f.edit(Edit::SkillDraft {
        name: "review",
        markdown: &current.markdown,
        metadata: &current.metadata,
        markdown_hash: &current.markdown_hash,
        metadata_hash: &current.metadata_hash,
    });
    assert_eq!(f.head(), head);
    assert_eq!(f.remote_file("skills/review/skill.toml"), "");
}

#[test]
fn desktop_covers_skill_lifecycle_resources_configuration_and_agent_targets() {
    let f = Fixture::new();
    f.edit(Edit::Create {
        name: "new-skill",
        description: "A new skill",
    });
    let import = f.root.join("resource.md");
    fs::write(&import, "Resource bytes\n").unwrap();
    f.edit(Edit::AddResource {
        name: "new-skill",
        file: "references/[guide].md",
        from: &import,
    });
    assert_eq!(
        f.remote_file("skills/new-skill/references/[guide].md"),
        "Resource bytes"
    );
    let variant = f.root.join("variant.md");
    fs::write(
        &variant,
        "---\nname: new-skill\ndescription: Windows skill\n---\nWindows instructions\n",
    )
    .unwrap();
    f.edit(Edit::AddResource {
        name: "new-skill",
        file: "variants/os=windows/SKILL.md",
        from: &variant,
    });
    f.edit(Edit::SetEnabled {
        name: "new-skill",
        enabled: false,
    });
    assert!(
        f.remote_file("skills/new-skill/skill.toml")
            .contains("enabled = false")
    );
    let document = core::read_skill(&f.settings, None, "new-skill").unwrap();
    f.edit(Edit::Rename {
        name: "new-skill",
        new_name: "renamed",
        expected_hash: &document.markdown_hash,
    });
    assert!(
        f.remote_file("skills/renamed/variants/os=windows/SKILL.md")
            .contains("renamed")
    );
    assert!(
        !output(
            &f.remote(),
            &["cat-file", "-e", "main:skills/new-skill/SKILL.md"]
        )
        .status
        .success()
    );
    let archived = f.edit(Edit::Archive { name: "renamed" }).unwrap();
    let archived = archived
        .strip_prefix(&f.settings.source)
        .unwrap()
        .to_str()
        .unwrap()
        .replace('\\', "/");
    assert_eq!(
        f.remote_file(&format!("{archived}/references/[guide].md")),
        "Resource bytes"
    );
    assert!(
        !output(
            &f.remote(),
            &["cat-file", "-e", "main:skills/renamed/SKILL.md"]
        )
        .status
        .success()
    );
    for file in ["machines/test.toml", "harnesses.toml"] {
        let mut document = core::structured_source(&f.settings, file).unwrap();
        if file.starts_with("machines") {
            document
                .values
                .insert("capabilities".into(), vec!["testing"].into());
        } else {
            document
                .values
                .get_mut("codex")
                .unwrap()
                .as_table_mut()
                .unwrap()
                .insert("project_root".into(), ".codex-tools".into());
        }
        f.edit(Edit::StructuredSource {
            file,
            hash: &document.hash,
            values: document.values,
        });
        assert!(
            f.remote_file(file)
                .contains(if file.starts_with("machines") {
                    "testing"
                } else {
                    ".codex-tools"
                })
        );
    }
    let hash = core::read_source_file(&f.settings, "harnesses.toml")
        .unwrap()
        .hash;
    let target = core::agent_presets()
        .into_iter()
        .find(|p| p.target.tag == "gemini")
        .unwrap()
        .target;
    f.edit(Edit::AddAgentTarget {
        target: &target,
        expected_hash: &hash,
    });
    assert!(f.remote_file("harnesses.toml").contains("[gemini]"));
    assert!(!core::content_status(&f.settings).needs_attention());
}

#[test]
fn clean_checkout_fast_forwards_before_hash_checks_and_keeps_remote_changes() {
    let f = Fixture::new();
    let original = core::read_global_instructions(&f.settings).unwrap();
    let skill = core::read_skill(&f.settings, None, "review").unwrap();
    let peer = f.clone_peer();
    fs::write(peer.join("global/AGENTS.md"), "Changed remotely\n").unwrap();
    git(&peer, &["add", "."]);
    git(&peer, &["commit", "-m", "Remote edit"]);
    assert_eq!(
        git(
            &peer,
            &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"]
        ),
        "global/AGENTS.md",
        "The peer must not change unrelated files through line-ending conversion"
    );
    git(&peer, &["push"]);
    let error = core::desktop_source_edit(
        &f.settings,
        None,
        Edit::GlobalInstructions {
            markdown: "Stale editor\n",
            expected_hash: &original.hash,
        },
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Source changed; reload before saving"),
        "{error:#}"
    );
    assert_eq!(
        fs::read_to_string(f.settings.source.join("global/AGENTS.md")).unwrap(),
        "Changed remotely\n"
    );
    assert_eq!(f.head(), git(&peer, &["rev-parse", "HEAD"]));
    let markdown = skill
        .markdown
        .replace("Review the changes.", "Review carefully.");
    f.edit(Edit::SkillDraft {
        name: "review",
        markdown: &markdown,
        metadata: &skill.metadata,
        markdown_hash: &skill.markdown_hash,
        metadata_hash: &skill.metadata_hash,
    });
    assert_eq!(f.remote_file("global/AGENTS.md"), "Changed remotely");
    // A disjoint remote update is pulled before a successful save too.
    fs::write(peer.join("notes.txt"), "New remote notes\n").unwrap();
    git(&peer, &["add", "."]);
    git(&peer, &["commit", "-m", "Remote notes"]);
    git(&peer, &["pull", "--rebase"]);
    git(&peer, &["push"]);
    f.save_global("Fresh local instructions\n").unwrap();
    assert_eq!(f.remote_file("notes.txt"), "New remote notes");
}

#[test]
fn commit_failure_leaves_saved_file_in_place_and_reports_it() {
    let f = Fixture::new();
    let original = f.head();
    f.hook("pre-commit", "echo 'Commit hook refused' >&2\nexit 1");
    let error = f.save_global("Saved despite failed commit\n").unwrap_err();
    let error = format!("{error:#}");
    assert!(
        error.contains("Source saved in place but Git commit failed"),
        "{error}"
    );
    assert!(error.contains("Commit hook refused"));
    assert_eq!(f.head(), original);
    assert_eq!(
        fs::read_to_string(f.settings.source.join("global/AGENTS.md")).unwrap(),
        "Saved despite failed commit\n"
    );
    assert_eq!(core::content_status(&f.settings).changed_files, 1);
}

#[test]
fn push_failure_keeps_local_commit_status_and_deduplicated_sync_alert_then_retry_recovers() {
    let mut f = Fixture::new();
    f.settings.alert_command = vec![
        "git".into(),
        "-C".into(),
        f.settings.source.display().to_string(),
        "config".into(),
        "--add".into(),
        "ratatoskr.alert".into(),
    ];
    core::save(&f.config, &f.settings).unwrap();
    git(
        &f.settings.source,
        &["config", "remote.origin.pushurl", "../offline.git"],
    );
    let error = f.save_global("Local commit awaiting push\n").unwrap_err();
    assert!(format!("{error:#}").contains("saved and committed locally but NOT pushed"));
    let status = core::content_status(&f.settings);
    assert_eq!(status.changed_files, 0);
    assert_eq!(status.outgoing_commits, Some(1));
    assert!(status.needs_attention());
    assert_eq!(
        core::plan(&f.settings)
            .unwrap()
            .content_status
            .outgoing_commits,
        Some(1)
    );
    core::apply(&f.settings).unwrap();
    assert!(core::plan(&f.settings).unwrap().changes.is_empty());
    let report = f.cli(&["--json", "status"]);
    assert_eq!(
        report.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&report.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&report.stdout).unwrap();
    assert_eq!(report["content_status"]["outgoing_commits"], 1);
    for _ in 0..2 {
        let error = core::sync(&f.settings).unwrap_err();
        assert!(
            format!("{error:#}").contains("1 unpushed commits"),
            "{error:#}"
        );
    }
    let alerts = git(
        &f.settings.source,
        &["config", "--get-all", "ratatoskr.alert"],
    );
    assert_eq!(alerts.lines().count(), 1);
    assert!(alerts.contains("unpushed commits"));
    let ledger = core::job_status(&f.settings).unwrap();
    assert!(ledger.current_error.unwrap().contains("unpushed commits"));
    assert!(
        ledger
            .last_run
            .unwrap()
            .error
            .unwrap()
            .contains("unpushed commits")
    );
    // Push now publishes commits only, preserving unrelated dirty/staged files.
    fs::write(
        f.settings.source.join("notes.txt"),
        "Unrelated pending notes",
    )
    .unwrap();
    git(&f.settings.source, &["add", "notes.txt"]);
    git(
        &f.settings.source,
        &["config", "--unset", "remote.origin.pushurl"],
    );
    core::push_content(&f.settings).unwrap();
    assert_eq!(core::content_status(&f.settings).outgoing_commits, Some(0));
    assert_eq!(core::content_status(&f.settings).changed_files, 1);
    assert_eq!(
        f.remote_file("global/AGENTS.md"),
        "Local commit awaiting push"
    );
    assert_eq!(f.remote_file("notes.txt"), "Original notes");
}

#[test]
fn cli_skill_actions_remain_write_only() {
    let f = Fixture::new();
    let head = f.head();
    let input = f.root.join("draft.md");
    fs::write(
        &input,
        core::new_skill_markdown("cli-skill", "CLI skill").unwrap(),
    )
    .unwrap();
    for args in [
        vec![
            "skill",
            "create",
            "cli-skill",
            "--input",
            input.to_str().unwrap(),
        ],
        vec!["skill", "disable", "cli-skill"],
    ] {
        let result = f.cli(&args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(f.head(), head);
    }
    let document = core::read_skill(&f.settings, None, "cli-skill").unwrap();
    fs::write(
        &input,
        document.markdown.replace("CLI skill", "CLI updated"),
    )
    .unwrap();
    let result = f.cli(&[
        "skill",
        "write",
        "cli-skill",
        "--file",
        "SKILL.md",
        "--input",
        input.to_str().unwrap(),
        "--expected-hash",
        &document.markdown_hash,
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = f.cli(&["skill", "archive", "cli-skill"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(f.head(), head);
    assert_eq!(git(&f.remote(), &["rev-parse", "main"]), head);
    assert!(core::content_status(&f.settings).changed_files > 0);
    assert_eq!(f.cli(&["status"]).status.code(), Some(1));
}

#[test]
fn monitored_project_desktop_edits_never_commit_fetch_or_push_central_or_project() {
    let mut f = Fixture::new();
    let project = f.root.join("project");
    fs::create_dir_all(project.join("agent-skills/skills")).unwrap();
    git(&project, &["init", "-b", "feature"]);
    core::add_project(&f.config, &project, "agent-skills").unwrap();
    f.settings = core::load(&f.config).unwrap();
    git(&f.settings.source, &["switch", "-c", "agent-feature"]);
    let head = f.head();
    let run = |edit| core::desktop_source_edit(&f.settings, Some(&project), edit).unwrap();
    run(Edit::Create {
        name: "project-skill",
        description: "Project skill",
    });
    let doc = core::read_skill(&f.settings, Some(&project), "project-skill").unwrap();
    let markdown = doc
        .markdown
        .replace("Project skill", "Changed project skill");
    run(Edit::SkillDraft {
        name: "project-skill",
        markdown: &markdown,
        metadata: &doc.metadata,
        markdown_hash: &doc.markdown_hash,
        metadata_hash: &doc.metadata_hash,
    });
    run(Edit::SetEnabled {
        name: "project-skill",
        enabled: false,
    });
    let doc = core::read_skill(&f.settings, Some(&project), "project-skill").unwrap();
    run(Edit::Rename {
        name: "project-skill",
        new_name: "renamed-project",
        expected_hash: &doc.markdown_hash,
    });
    let resource = f.root.join("project-resource.txt");
    fs::write(&resource, "Project resource").unwrap();
    run(Edit::AddResource {
        name: "renamed-project",
        file: "resource.txt",
        from: &resource,
    });
    run(Edit::Archive {
        name: "renamed-project",
    });
    assert_eq!(f.head(), head);
    assert!(!f.settings.source.join(".git/FETCH_HEAD").exists());
    assert!(
        !output(&project, &["rev-parse", "--verify", "HEAD"])
            .status
            .success()
    );
    assert!(core::project_status(&project).changed_files > 0);
}

#[test]
fn source_conflict_copy_back_edit_and_backup_restore_are_published() {
    for action in [
        core::ConflictAction::CopyBack,
        core::ConflictAction::EditSource,
    ] {
        let f = Fixture::new();
        core::apply(&f.settings).unwrap();
        let installed = f.settings.state_dir.join("test/staging/codex/AGENTS.md");
        fs::write(&installed, "Installed hand edit\n").unwrap();
        let preview = if action == core::ConflictAction::CopyBack {
            core::preview_resolution(&f.settings, &installed, action).unwrap()
        } else {
            core::preview_source_edit(&f.settings, &installed, "Edited source text\n").unwrap()
        };
        let expected = serde_json::to_string(&preview).unwrap();
        let backup = f
            .edit(Edit::ResolveConflict {
                expected: &expected,
            })
            .unwrap();
        assert_eq!(
            f.remote_file("global/AGENTS.md"),
            if action == core::ConflictAction::CopyBack {
                "Installed hand edit"
            } else {
                "Edited source text"
            }
        );
        // Restore refuses current output conflicts; adopting keeps installed bytes
        // and makes the backup's original source safe to restore.
        if action == core::ConflictAction::EditSource {
            let preview =
                core::preview_resolution(&f.settings, &installed, core::ConflictAction::Adopt)
                    .unwrap();
            f.edit(Edit::ResolveConflict {
                expected: &serde_json::to_string(&preview).unwrap(),
            });
        }
        f.edit(Edit::RestoreBackup { directory: &backup });
        assert_eq!(
            f.remote_file("global/AGENTS.md"),
            "Original global instructions"
        );
    }
}

#[test]
fn commit_and_push_hooks_observe_both_locks_held() {
    let f = Fixture::new();
    fn quote(path: &Path) -> String {
        format!(
            "'{}'",
            path.to_str()
                .unwrap()
                .replace('\\', "/")
                .replace('\'', "'\\''")
        )
    }
    let cli = quote(Path::new(env!("CARGO_BIN_EXE_ratatoskr")));
    let config = quote(&f.config);
    let log = quote(&f.root.join("hook.log"));
    let script = format!(
        "{cli} --config {config} sync > {log} 2>&1\ncase \"$(cat {log})\" in *sync.lock*) ;; *) exit 1 ;; esac\n{cli} --config {config} apply > {log} 2>&1\ncase \"$(cat {log})\" in *apply.lock*) ;; *) exit 1 ;; esac\nexit 0"
    );
    f.hook("pre-commit", &script);
    f.hook("pre-push", &script);
    f.save_global("Save locked through commit and push\n")
        .unwrap();
    assert_eq!(f.head(), git(&f.remote(), &["rev-parse", "main"]));
}

#[test]
fn dirty_behind_checkout_keeps_other_drafts_and_retains_a_rejected_local_commit() {
    let f = Fixture::new();
    let peer = f.clone_peer();
    fs::write(peer.join("global/AGENTS.md"), "Remote instructions\n").unwrap();
    git(&peer, &["add", "."]);
    git(&peer, &["commit", "-m", "Remote edit"]);
    git(&peer, &["push"]);
    fs::write(
        f.settings.source.join("notes.txt"),
        "Unrelated dirty notes\n",
    )
    .unwrap();
    let error = f.save_global("Local instructions\n").unwrap_err();
    assert!(format!("{error:#}").contains("saved and committed locally but NOT pushed"));
    assert_eq!(core::content_status(&f.settings).outgoing_commits, Some(1));
    assert_eq!(core::content_status(&f.settings).changed_files, 1);
    assert_eq!(
        fs::read_to_string(f.settings.source.join("notes.txt")).unwrap(),
        "Unrelated dirty notes\n"
    );
    assert_eq!(f.remote_file("global/AGENTS.md"), "Remote instructions");
    assert_eq!(
        git(&f.settings.source, &["show", "HEAD:global/AGENTS.md"]),
        "Local instructions"
    );
}

#[test]
fn desktop_refuses_wrong_branch_missing_upstream_and_failed_fetch_before_writing() {
    let f = Fixture::new();
    let original = fs::read(f.settings.source.join("global/AGENTS.md")).unwrap();
    git(&f.settings.source, &["switch", "-c", "agent-feature"]);
    assert!(format!("{:#}", f.save_global("Draft\n").unwrap_err()).contains("deploy_branch"));
    assert!(!f.settings.source.join(".git/FETCH_HEAD").exists());
    git(&f.settings.source, &["switch", "main"]);
    git(&f.settings.source, &["branch", "--unset-upstream"]);
    assert!(f.save_global("Draft\n").is_err());
    assert!(core::content_status(&f.settings).error.is_some());
    git(
        &f.settings.source,
        &["branch", "--set-upstream-to=origin/main"],
    );
    git(
        &f.settings.source,
        &["remote", "set-url", "origin", "../missing.git"],
    );
    let error = f.save_global("Draft\n").unwrap_err();
    assert!(format!("{error:#}").contains("source was not written"));
    assert_eq!(
        fs::read(f.settings.source.join("global/AGENTS.md")).unwrap(),
        original
    );
}

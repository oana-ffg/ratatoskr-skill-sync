//! This binary holds one test because it replaces `git` on the process-wide PATH.
#![cfg(unix)]
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn run_git(directory: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A repository with a pushed `main`, tracking its own bare remote.
fn repository(root: &Path, name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let remote = root.join(format!("{name}.git"));
    let checkout = root.join(name);
    fs::create_dir_all(&checkout).unwrap();
    run_git(
        root,
        &["init", "--bare", "-b", "main", &format!("{name}.git")],
    );
    run_git(&checkout, &["init", "-b", "main"]);
    run_git(&checkout, &["config", "user.email", "test@example.com"]);
    run_git(&checkout, &["config", "user.name", "Test"]);
    for (path, text) in files {
        let path = checkout.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    run_git(&checkout, &["add", "-A"]);
    run_git(&checkout, &["commit", "-m", "Initial"]);
    run_git(
        &checkout,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    run_git(&checkout, &["push", "-u", "origin", "main"]);
    checkout
}

/// A plan runs on every workspace refresh and every minute in the background. It used to spawn
/// 9 processes for the content checkout and 8 per project, because each project's status was
/// computed twice. It needs 4, plus 2 per project with no outgoing commits.
#[test]
fn a_plan_spawns_a_bounded_number_of_git_processes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = repository(
        &root,
        "content",
        &[
            (
                "harnesses.toml",
                "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\nroots={linux='/x',macos='/x',windows='C:/x'}\n",
            ),
            (
                "machines/test.toml",
                "os='macos'\nmode='staging'\nharnesses=['codex']\n",
            ),
            (
                "skills/central/SKILL.md",
                "---\nname: central\ndescription: Central\n---\nBody.\n",
            ),
            ("skills/central/skill.toml", ""),
        ],
    );
    let projects: Vec<_> = ["first", "second", "third"]
        .iter()
        .map(|name| {
            let skill = format!("---\nname: {name}\ndescription: Project\n---\nBody.\n");
            repository(
                &root,
                name,
                &[
                    (".ratatoskr.toml", "source = 'agent-skills'\n"),
                    (&format!("agent-skills/skills/{name}/SKILL.md"), &skill),
                    (&format!("agent-skills/skills/{name}/skill.toml"), ""),
                ],
            )
        })
        .collect();
    let settings = ratatoskr_core::Settings {
        source,
        state_dir: root.join("state"),
        machine: "test".into(),
        projects: projects.clone(),
        ..Default::default()
    };
    let real_git = String::from_utf8(Command::new("which").arg("git").output().unwrap().stdout)
        .unwrap()
        .trim()
        .to_owned();
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let count = root.join("git-invocations");
    let wrapper = bin.join("git");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\nexec '{real_git}' \"$@\"\n",
            count.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::var_os("PATH").unwrap();
    let mut search = vec![bin];
    search.extend(std::env::split_paths(&path));
    // SAFETY: this is the only test in this binary, so no other thread reads the environment.
    unsafe {
        std::env::set_var("PATH", std::env::join_paths(search).unwrap());
    }
    let plan = ratatoskr_core::plan(&settings).unwrap();
    assert!(
        plan.content_status.error.is_none(),
        "{:?}",
        plan.content_status
    );
    assert_eq!(plan.content_status.outgoing_commits, Some(0));
    for project in &plan.projects {
        assert!(project.error.is_none(), "{project:?}");
        assert_eq!(project.outgoing_commits, Some(0));
        assert_eq!(project.upstream.as_deref(), Some("origin/main"));
    }
    let invocations = fs::read_to_string(&count).unwrap();
    let spawned = invocations.lines().count();
    assert!(
        spawned <= 4 + 2 * projects.len(),
        "{spawned} Git processes for one plan:\n{invocations}"
    );
}

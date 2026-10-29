//! End-to-end journeys through the real `ratatoskr` binary. An author checkout and
//! independent deployment checkouts share a local bare Git remote. Every machine
//! profile uses staging, so outputs stay beneath a disposable state directory and
//! no real harness folder is touched.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const HOST_OS: &str = std::env::consts::OS;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Asserts the exit code and returns stdout.
fn expect(output: Output, code: i32) -> String {
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

/// Rendered text with line endings normalized: a Windows clone may check out CRLF.
fn text(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("Read {}: {error}", path.display()))
        .replace("\r\n", "\n")
}

fn skill(name: &str, body: &str) -> String {
    format!("---\nname: {name}\ndescription: {name} skill\n---\n{body}")
}

struct Workspace {
    _temp: tempfile::TempDir,
    root: PathBuf,
    remote: PathBuf,
    author: PathBuf,
}

impl Workspace {
    /// Publishes two deliberately different profiles: `alpha` runs on this host's OS
    /// with Claude and Codex; `beta` is a Codex-only profile for another OS.
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let remote = root.join("remote.git");
        fs::create_dir(&remote).unwrap();
        git(&remote, &["init", "--bare", "-b", "main"]);
        let author = root.join("author");
        let beta_os = if HOST_OS == "windows" {
            "linux"
        } else {
            "windows"
        };
        let mut harnesses = String::new();
        for (name, global_file, root) in [
            ("claude", "CLAUDE.md", ".claude"),
            ("codex", "AGENTS.md", ".agents"),
        ] {
            harnesses.push_str(&format!(
                "[{name}]\nskills_dir='skills'\nglobal_file='{global_file}'\n[{name}.roots]\n"
            ));
            for os in ["macos", "windows", "linux"] {
                harnesses.push_str(&format!("{os}='{{{{af:home}}}}/{root}'\n"));
            }
        }
        let files = [
            ("harnesses.toml", harnesses),
            (
                "machines/alpha.toml",
                format!(
                    "os='{HOST_OS}'\nmode='staging'\nharnesses=['claude','codex']\n[paths]\narchive='/archive'\n"
                ),
            ),
            (
                "machines/beta.toml",
                format!(
                    "os='{beta_os}'\nmode='staging'\nharnesses=['codex']\n[paths]\narchive='C:\\Archive'\n"
                ),
            ),
            (
                "global/AGENTS.md",
                "Shared rules for {{af:machine}}.\n".into(),
            ),
            (
                "skills/review/SKILL.md",
                skill(
                    "review",
                    &format!(
                        "{{{{af:machine}}}} archives to {{{{af:paths.archive}}}}.\n<!-- only harness=codex -->\nCodex only.\n<!-- /only -->\n<!-- only os={HOST_OS} -->\nRuns on {HOST_OS}.\n<!-- /only -->\n"
                    ),
                ),
            ),
            ("skills/review/skill.toml", String::new()),
            (
                "skills/claude-only/SKILL.md",
                skill("claude-only", "Claude.\n"),
            ),
            (
                "skills/claude-only/skill.toml",
                "[target]\nharnesses=['claude']\n".into(),
            ),
            ("skills/needs-gpu/SKILL.md", skill("needs-gpu", "GPU.\n")),
            (
                "skills/needs-gpu/skill.toml",
                "[target]\ncapabilities=['gpu']\n".into(),
            ),
        ];
        for (path, contents) in files {
            let path = author.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        git(&author, &["init", "-b", "main"]);
        for (key, value) in [
            ("user.name", "Ratatoskr author"),
            ("user.email", "author@ratatoskr.invalid"),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", ".git/hooks"),
        ] {
            git(&author, &["config", key, value]);
        }
        git(&author, &["add", "."]);
        git(&author, &["commit", "-m", "Initial content"]);
        git(
            &author,
            &["remote", "add", "origin", &remote.to_string_lossy()],
        );
        git(&author, &["push", "-u", "origin", "main"]);
        Self {
            _temp: temp,
            root,
            remote,
            author,
        }
    }

    /// Commits and pushes an author change, as another machine or the desktop would.
    fn publish(&self, message: &str, change: impl FnOnce(&Path)) {
        change(&self.author);
        git(&self.author, &["add", "--all"]);
        git(&self.author, &["commit", "-m", message]);
        git(&self.author, &["push"]);
    }

    /// Clones the remote into a dedicated deployment checkout. Nothing is applied yet.
    fn deploy(&self, machine: &str) -> Deployment {
        let directory = self.root.join(machine);
        let deployment = Deployment {
            config: directory.join("config.toml"),
            state: directory.join("state"),
            machine: machine.into(),
        };
        expect(
            deployment.ratatoskr(&[
                "init",
                "--remote",
                &self.remote.to_string_lossy(),
                "--source",
                &directory.join("content").to_string_lossy(),
                "--machine",
                machine,
                "--state-dir",
                &deployment.state.to_string_lossy(),
            ]),
            0,
        );
        // Keep every sync offline and deterministic: the helper's automatic
        // self-update would otherwise query the public release endpoint.
        let mut settings = ratatoskr_core::load(&deployment.config).unwrap();
        settings.updates.automatic = false;
        ratatoskr_core::save(&deployment.config, &settings).unwrap();
        deployment
    }
}

struct Deployment {
    config: PathBuf,
    state: PathBuf,
    machine: String,
}

impl Deployment {
    fn ratatoskr(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ratatoskr"))
            .env_remove("RATATOSKR_CONFIG")
            .arg("--config")
            .arg(&self.config)
            .args(args)
            .output()
            .unwrap()
    }

    fn staged(&self, path: &str) -> PathBuf {
        self.state.join(&self.machine).join("staging").join(path)
    }

    fn job_status(&self) -> serde_json::Value {
        serde_json::from_str(&expect(self.ratatoskr(&["job-status"]), 0)).unwrap()
    }
}

#[test]
fn deployments_render_profile_differences_and_follow_published_changes() {
    let workspace = Workspace::new();
    let validation: serde_json::Value = serde_json::from_str(&expect(
        Command::new(env!("CARGO_BIN_EXE_ratatoskr"))
            .args(["--json", "validate"])
            .arg(&workspace.author)
            .output()
            .unwrap(),
        0,
    ))
    .unwrap();
    assert_eq!(validation["profiles"].as_array().unwrap().len(), 2);
    assert_eq!(validation["problems"], serde_json::json!([]));

    let alpha = workspace.deploy("alpha");
    // Pending output is reported, not written, until apply.
    expect(alpha.ratatoskr(&["status"]), 1);
    assert!(!alpha.staged("codex").exists());
    let applied = expect(alpha.ratatoskr(&["apply"]), 0);
    assert!(
        applied.contains("Applied 5 changes: 5 added, 0 updated, 0 removed."),
        "{applied}"
    );
    expect(alpha.ratatoskr(&["status"]), 0);
    for harness in ["claude", "codex"] {
        let review = text(&alpha.staged(&format!("{harness}/skills/review/SKILL.md")));
        assert!(review.contains("alpha archives to /archive."), "{review}");
        assert!(review.contains(&format!("Runs on {HOST_OS}.")), "{review}");
        assert_eq!(
            review.contains("Codex only."),
            harness == "codex",
            "{review}"
        );
        assert!(
            !alpha
                .staged(&format!("{harness}/skills/needs-gpu"))
                .exists()
        );
    }
    assert_eq!(
        text(&alpha.staged("claude/CLAUDE.md")),
        "Shared rules for alpha.\n"
    );
    assert_eq!(
        text(&alpha.staged("codex/AGENTS.md")),
        "Shared rules for alpha.\n"
    );
    assert!(alpha.staged("claude/skills/claude-only/SKILL.md").is_file());
    assert!(!alpha.staged("codex/skills/claude-only").exists());

    let beta = workspace.deploy("beta");
    expect(beta.ratatoskr(&["apply"]), 0);
    let review = text(&beta.staged("codex/skills/review/SKILL.md"));
    assert!(review.contains("beta archives to C:\\Archive."), "{review}");
    assert!(review.contains("Codex only."), "{review}");
    assert!(!review.contains("Runs on"), "{review}");
    assert!(!beta.staged("claude").exists());

    workspace.publish("Revise skills", |author| {
        fs::remove_dir_all(author.join("skills/claude-only")).unwrap();
        fs::create_dir(author.join("skills/notes")).unwrap();
        fs::write(
            author.join("skills/notes/SKILL.md"),
            skill("notes", "Notes.\n"),
        )
        .unwrap();
        fs::write(author.join("skills/notes/skill.toml"), "").unwrap();
        fs::write(
            author.join("global/AGENTS.md"),
            "Revised rules for {{af:machine}}.\n",
        )
        .unwrap();
    });

    expect(alpha.ratatoskr(&["sync"]), 0);
    assert!(alpha.staged("claude/skills/notes/SKILL.md").is_file());
    assert!(alpha.staged("codex/skills/notes/SKILL.md").is_file());
    assert!(!alpha.staged("claude/skills/claude-only").exists());
    assert_eq!(
        text(&alpha.staged("codex/AGENTS.md")),
        "Revised rules for alpha.\n"
    );
    let status = alpha.job_status();
    assert_eq!(
        status["last_run"]["error"],
        serde_json::Value::Null,
        "{status}"
    );
    assert!(status["last_run"]["finished_at"].is_u64(), "{status}");
    assert!(
        status["log"].as_str().unwrap().contains("Sync succeeded"),
        "{status}"
    );

    // Each deployment only changes when it syncs itself.
    assert!(!beta.staged("codex/skills/notes").exists());
    expect(beta.ratatoskr(&["sync"]), 0);
    assert!(beta.staged("codex/skills/notes/SKILL.md").is_file());
    assert_eq!(
        text(&beta.staged("codex/AGENTS.md")),
        "Revised rules for beta.\n"
    );
}

#[test]
fn hand_edits_block_sync_until_explicitly_resolved() {
    let workspace = Workspace::new();
    let alpha = workspace.deploy("alpha");
    expect(alpha.ratatoskr(&["apply"]), 0);
    let edited = alpha.staged("codex/skills/review/SKILL.md");
    let rendered = text(&edited);
    fs::write(&edited, format!("{rendered}Local hand edit.\n")).unwrap();
    workspace.publish("Add notes", |author| {
        fs::create_dir(author.join("skills/notes")).unwrap();
        fs::write(
            author.join("skills/notes/SKILL.md"),
            skill("notes", "Notes.\n"),
        )
        .unwrap();
        fs::write(author.join("skills/notes/skill.toml"), "").unwrap();
    });

    let refused = alpha.ratatoskr(&["sync"]);
    let stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
    expect(refused, 2);
    assert!(
        stderr.contains("Refusing all writes because of conflicts"),
        "{stderr}"
    );
    assert!(text(&edited).ends_with("Local hand edit.\n"));
    assert!(!alpha.staged("codex/skills/notes").exists());
    let plan: serde_json::Value =
        serde_json::from_str(&expect(alpha.ratatoskr(&["--json", "plan"]), 1)).unwrap();
    assert_eq!(plan["conflicts"].as_array().unwrap().len(), 1, "{plan}");
    let status = alpha.job_status();
    assert!(status["last_run"]["error"].is_string(), "{status}");

    let preview = expect(
        alpha.ratatoskr(&[
            "conflict",
            "preview",
            &edited.to_string_lossy(),
            "--action",
            "reinstall",
        ]),
        0,
    );
    let preview_file = workspace.root.join("reinstall.json");
    fs::write(&preview_file, preview).unwrap();
    let resolved = expect(
        alpha.ratatoskr(&[
            "conflict",
            "resolve",
            "--preview",
            &preview_file.to_string_lossy(),
        ]),
        0,
    );
    let backup = PathBuf::from(resolved.trim().strip_prefix("Verified backup: ").unwrap());
    assert!(backup.is_dir(), "{resolved}");
    assert_eq!(text(&edited), rendered);

    expect(alpha.ratatoskr(&["sync"]), 0);
    assert!(alpha.staged("codex/skills/notes/SKILL.md").is_file());
    expect(alpha.ratatoskr(&["status"]), 0);
}

#[cfg(target_os = "macos")]
mod launchd {
    use super::*;
    use std::time::{Duration, Instant};

    /// Polls until `done` holds, failing with the deployment's job status and scheduler logs.
    fn wait_for(deployment: &Deployment, what: &str, timeout: Duration, done: impl Fn() -> bool) {
        let start = Instant::now();
        while !done() {
            if start.elapsed() > timeout {
                let logs = deployment.state.join(&deployment.machine);
                panic!(
                    "Timed out after {timeout:?} waiting for {what}.\njob-status: {}\nstdout log: {}\nstderr log: {}",
                    deployment.job_status(),
                    fs::read_to_string(logs.join("sync.stdout.log")).unwrap_or_default(),
                    fs::read_to_string(logs.join("sync.stderr.log")).unwrap_or_default(),
                );
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    /// Successful runs in the job history. The ledger records `last_run` before it
    /// appends history, so waiting on the history covers the whole record.
    fn successes(deployment: &Deployment) -> usize {
        deployment.job_status()["log"]
            .as_str()
            .unwrap()
            .matches("Sync succeeded")
            .count()
    }

    /// Boots out and deletes the job if the test fails before `schedule remove` does.
    /// Armed from the plan, so a failure partway through installation is covered too.
    struct LaunchdCleanup {
        target: String,
        plist: PathBuf,
    }

    impl LaunchdCleanup {
        fn planned(definition: &str) -> Self {
            let label = definition
                .split_once("<key>Label</key><string>")
                .and_then(|(_, rest)| rest.split_once("</string>"))
                .map(|(label, _)| label)
                .expect("Planned definition has no label");
            let uid = Command::new("id").arg("-u").output().unwrap();
            assert!(uid.status.success(), "Cannot determine login UID");
            Self {
                target: format!(
                    "gui/{}/{label}",
                    String::from_utf8(uid.stdout).unwrap().trim()
                ),
                plist: PathBuf::from(std::env::var_os("HOME").unwrap())
                    .join("Library/LaunchAgents")
                    .join(format!("{label}.plist")),
            }
        }

        fn loaded(&self) -> std::io::Result<bool> {
            let print = Command::new("launchctl")
                .args(["print", &self.target])
                .output()?;
            Ok(print.status.success())
        }
    }

    impl Drop for LaunchdCleanup {
        fn drop(&mut self) {
            match self.loaded() {
                Ok(false) => (),
                Ok(true) => {
                    let bootout = Command::new("launchctl")
                        .args(["bootout", &self.target])
                        .output();
                    if !matches!(self.loaded(), Ok(false)) {
                        eprintln!(
                            "WARNING: launchd job {} is still loaded after cleanup: {bootout:?}",
                            self.target
                        );
                    }
                }
                Err(error) => eprintln!(
                    "WARNING: cannot inspect launchd job {}: {error}",
                    self.target
                ),
            }
            if self.plist.exists()
                && let Err(error) = fs::remove_file(&self.plist)
            {
                eprintln!(
                    "WARNING: leftover launchd plist {}: {error}",
                    self.plist.display()
                );
            }
        }
    }

    #[test]
    #[ignore = "installs a real per-user launchd job; CI runs it with --ignored"]
    fn launchd_schedule_syncs_published_changes_with_no_app_running() {
        let workspace = Workspace::new();
        let alpha = workspace.deploy("alpha");
        let helper = dunce::canonicalize(env!("CARGO_BIN_EXE_ratatoskr")).unwrap();
        let helper = helper.to_string_lossy();
        let definition = expect(
            alpha.ratatoskr(&["schedule", "plan", "--helper", &helper, "--seconds", "60"]),
            0,
        );
        assert!(
            definition.contains(&format!("<string>{helper}</string>")),
            "{definition}"
        );
        assert!(definition.contains("<string>sync</string>"), "{definition}");
        assert!(definition.contains("<integer>60</integer>"), "{definition}");

        let cleanup = LaunchdCleanup::planned(&definition);
        expect(
            alpha.ratatoskr(&[
                "schedule",
                "install",
                "--helper",
                &helper,
                "--seconds",
                "60",
            ]),
            0,
        );
        let record: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(alpha.state.join("alpha/installed-schedule.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            record["path"].as_str().map(PathBuf::from),
            Some(cleanup.plist.clone())
        );
        assert!(
            cleanup
                .target
                .ends_with(&format!("/{}", record["label"].as_str().unwrap()))
        );
        assert_eq!(
            text(&cleanup.plist),
            definition.trim_end_matches('\n').to_owned() + "\n"
        );
        assert!(
            cleanup.loaded().unwrap(),
            "launchd did not load {}",
            cleanup.target
        );

        // RunAtLoad performs the first sync; the interval picks up later publications.
        wait_for(
            &alpha,
            "the load-time sync",
            Duration::from_secs(90),
            || alpha.staged("codex/skills/review/SKILL.md").is_file() && successes(&alpha) >= 1,
        );
        let before = successes(&alpha);
        workspace.publish("Add notes", |author| {
            fs::create_dir(author.join("skills/notes")).unwrap();
            fs::write(
                author.join("skills/notes/SKILL.md"),
                skill("notes", "Notes.\n"),
            )
            .unwrap();
            fs::write(author.join("skills/notes/skill.toml"), "").unwrap();
        });
        wait_for(
            &alpha,
            "the interval sync",
            Duration::from_secs(150),
            || alpha.staged("codex/skills/notes/SKILL.md").is_file() && successes(&alpha) > before,
        );

        expect(alpha.ratatoskr(&["schedule", "remove"]), 0);
        assert!(!cleanup.plist.exists());
        assert!(!alpha.state.join("alpha/installed-schedule.json").exists());
        assert!(
            !cleanup.loaded().unwrap(),
            "launchd still has {}",
            cleanup.target
        );
    }
}

use ratatoskr_core::{Settings, load};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        Self { _temp: temp, root }
    }

    fn source(&self) -> PathBuf {
        let source = self.root.join("existing-content");
        fs::create_dir_all(source.join("machines")).unwrap();
        fs::create_dir(source.join("skills")).unwrap();
        fs::write(source.join("harnesses.toml"), "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\n[codex.roots]\nmacos='{{af:home}}/.agents'\nwindows='{{af:home}}/.agents'\nlinux='{{af:home}}/.agents'\n").unwrap();
        fs::write(
            source.join("machines/test.toml"),
            format!(
                "os='{}'\nmode='staging'\nharnesses=['codex']\n",
                std::env::consts::OS
            ),
        )
        .unwrap();
        source
    }

    fn init(&self, mode: &str, explicit_state: bool, environment_config: bool) -> Settings {
        let config = self.root.join("preview/config.toml");
        let state = self.root.join("chosen-state");
        let mut command = Command::new(env!("CARGO_BIN_EXE_ratatoskr"));
        command.env_remove("RATATOSKR_CONFIG");
        if environment_config {
            command.env("RATATOSKR_CONFIG", &config);
        } else {
            command.arg("--config").arg(&config);
        }
        command.arg("init").args(["--machine", "test"]);
        let source = match mode {
            "create" => {
                command.arg("--create");
                self.root.join("new-content")
            }
            "remote" => {
                let source = self.source();
                git(&source, &["init", "-b", "main"]);
                git(&source, &["add", "."]);
                git(
                    &source,
                    &[
                        "-c",
                        "user.name=Ratatoskr tests",
                        "-c",
                        "user.email=tests@ratatoskr.invalid",
                        "-c",
                        "commit.gpgsign=false",
                        "commit",
                        "-m",
                        "Test content",
                    ],
                );
                command.arg("--remote").arg(source);
                self.root.join("cloned-content")
            }
            "connect" => self.source(),
            _ => panic!("Unknown setup mode: {mode}"),
        };
        command.arg("--source").arg(source);
        if explicit_state {
            command.arg("--state-dir").arg(&state);
        }
        assert_success(command.output().unwrap());
        let settings = load(&config).unwrap();
        assert_eq!(
            settings.state_dir,
            if explicit_state {
                state
            } else {
                self.root.join("preview/config.toml.state")
            }
        );
        assert!(!settings.state_dir.exists(), "Init must not write state");
        settings
    }
}

fn assert_success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git(source: &Path, args: &[&str]) {
    assert_success(
        Command::new("git")
            .current_dir(source)
            .args(args)
            .output()
            .unwrap(),
    );
}

#[test]
fn custom_config_isolates_state_for_connect_create_and_clone() {
    for mode in ["connect", "create", "remote"] {
        Fixture::new().init(mode, false, false);
    }
}

#[test]
fn explicit_state_dir_is_used_for_connect_create_and_clone() {
    for mode in ["connect", "create", "remote"] {
        Fixture::new().init(mode, true, false);
    }
}

#[test]
fn environment_config_override_also_isolates_state() {
    Fixture::new().init("connect", false, true);
}

#[test]
fn existing_config_keeps_its_explicit_state_and_bytes() {
    let fixture = Fixture::new();
    let settings = fixture.init("connect", true, false);
    let config = fixture.root.join("preview/config.toml");
    let before = fs::read(&config).unwrap();
    // Loading and planning must use the saved state even though it differs from
    // the new default beside this custom config.
    assert_success(
        Command::new(env!("CARGO_BIN_EXE_ratatoskr"))
            .arg("--config")
            .arg(&config)
            .arg("plan")
            .output()
            .unwrap(),
    );
    assert_eq!(load(&config).unwrap().state_dir, settings.state_dir);
    assert_eq!(fs::read(&config).unwrap(), before);
}

#[test]
fn init_help_explains_state_selection() {
    let output = Command::new(env!("CARGO_BIN_EXE_ratatoskr"))
        .args(["init", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--state-dir <DIR>"));
    assert!(help.contains("beside a custom config"));
}

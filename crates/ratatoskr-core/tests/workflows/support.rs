use ratatoskr_core::Settings;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(super) struct Fixture {
    _temp: tempfile::TempDir,
    pub(super) root: PathBuf,
    pub(super) settings: Settings,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self::with_temp(tempfile::tempdir().unwrap())
    }

    pub(super) fn with_temp(temp: tempfile::TempDir) -> Self {
        let root = temp.path().canonicalize().unwrap();
        let source = root.join("content");
        fs::create_dir_all(source.join("machines")).unwrap();
        fs::create_dir_all(source.join("skills/review-code")).unwrap();
        fs::write(source.join("harnesses.toml"), "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\n[codex.roots]\nmacos='{{af:home}}/.codex'\nwindows='{{af:home}}/.codex'\nlinux='{{af:home}}/.codex'\n").unwrap();
        fs::write(source.join("machines/test.toml"), "os='macos'\nmode='staging'\nharnesses=['codex']\ncapabilities=[]\n[paths]\nvault='/knowledge'\n").unwrap();
        fs::write(source.join("skills/review-code/skill.toml"), "").unwrap();
        fs::write(source.join("skills/review-code/SKILL.md"), "---\nname: review-code\ndescription: Review changed code\n---\nRead {{af:paths.vault}}.\n<!-- only harness=codex -->Report findings.<!-- /only -->\n<!-- only harness=claude -->{{af:missing}}<!-- /only -->\n").unwrap();
        let settings = Settings {
            source,
            machine: "test".into(),
            state_dir: root.join("state"),
            projects: vec![],
            ..Settings::default()
        };
        Self {
            _temp: temp,
            root,
            settings,
        }
    }
    pub(super) fn output(&self) -> PathBuf {
        self.settings
            .state_dir
            .join("test/staging/codex/skills/review-code")
    }
    pub(super) fn write(&self, path: &str, content: &str) {
        let path = self.settings.source.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
}

pub(super) fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(super) fn reviewed_resolution(
    f: &Fixture,
    path: &Path,
    action: ratatoskr_core::ConflictAction,
) -> String {
    serde_json::to_string(&ratatoskr_core::preview_resolution(&f.settings, path, action).unwrap())
        .unwrap()
}

/// Git rejects the verbatim `\\?\` paths that `canonicalize` returns on Windows.
pub(super) fn git_path(path: &Path) -> String {
    dunce::simplified(path).to_string_lossy().replace('\\', "/")
}

impl Fixture {
    pub(super) fn ownership(&self) -> serde_json::Value {
        serde_json::from_slice(
            &fs::read(self.settings.state_dir.join("test/manifest.json")).unwrap(),
        )
        .unwrap()
    }
}

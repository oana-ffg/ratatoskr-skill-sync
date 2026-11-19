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
    /// Keep live outputs inside the fixture while satisfying the host-home identity check.
    pub(super) fn presets(&self, tags: &[&str], mode: &str) {
        self.write("harnesses.toml", "");
        let fixture_root = |template: &str| {
            self.root
                .join("home")
                .join(template.strip_prefix("{{af:home}}/").unwrap())
                .to_string_lossy()
                .into_owned()
        };
        for tag in tags {
            let mut target = ratatoskr_core::agent_presets()
                .into_iter()
                .find(|preset| preset.target.tag == *tag)
                .unwrap()
                .target;
            target.global_folder = fixture_root(&target.global_folder);
            target.instructions_folder = target.instructions_folder.map(|root| fixture_root(&root));
            let preview = ratatoskr_core::preview_agent_target(&self.settings, &target).unwrap();
            self.write("harnesses.toml", &preview.text);
        }
        let actual_home = directories::BaseDirs::new()
            .unwrap()
            .home_dir()
            .to_path_buf();
        let profile = toml::Table::from_iter([
            ("os".into(), std::env::consts::OS.into()),
            ("mode".into(), mode.into()),
            (
                "home".into(),
                actual_home.to_string_lossy().into_owned().into(),
            ),
            (
                "harnesses".into(),
                toml::Value::Array(tags.iter().map(|tag| (*tag).into()).collect()),
            ),
        ]);
        self.write("machines/test.toml", &toml::to_string(&profile).unwrap());
        self.write(
            "skills/review-code/SKILL.md",
            "---\nname: review-code\ndescription: Review code\n---\nReview it.\n",
        );
        self.write("global/AGENTS.md", "Global instructions.\n");
    }

    pub(super) fn register_skill_project(&mut self) -> PathBuf {
        let project = self.root.join("project");
        let skill = project.join("agent-skills/skills/project-review");
        fs::create_dir_all(&skill).unwrap();
        git(&project, &["init", "-b", "main"]);
        fs::write(skill.join("skill.toml"), "").unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: project-review\ndescription: Review project\n---\nReview it.\n",
        )
        .unwrap();
        let config = self.root.join("config.toml");
        ratatoskr_core::save(&config, &self.settings).unwrap();
        ratatoskr_core::add_project(&config, &project, "agent-skills").unwrap();
        self.settings = ratatoskr_core::load(&config).unwrap();
        self.settings.projects[0].clone()
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

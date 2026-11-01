//! Guards against work that scales with the whole library, or that writes when nothing changed.
//! Structural checks come first because they cannot flake. The time budgets are deliberately
//! generous: they catch a change in kind, such as copying the library per keystroke.
use ratatoskr_core::{Settings, apply};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

struct Library {
    _temp: tempfile::TempDir,
    root: PathBuf,
    settings: Settings,
}

const SKILL: &str = "---\nname: review\ndescription: Review changes\n---\nReview changes.\n";

impl Library {
    /// `skills` real skills on three machine profiles, like a production library.
    fn new(skills: usize) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = root.join("content");
        fs::create_dir_all(source.join("machines")).unwrap();
        fs::write(
            source.join("harnesses.toml"),
            "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\nroots={linux='/x',macos='/x',windows='C:/x'}\n\
             [claude]\nskills_dir='skills'\nglobal_file='CLAUDE.md'\nroots={linux='/y',macos='/y',windows='C:/y'}\n",
        )
        .unwrap();
        for machine in ["test", "other", "third"] {
            fs::write(
                source.join("machines").join(format!("{machine}.toml")),
                "os='macos'\nmode='staging'\nharnesses=['codex','claude']\n",
            )
            .unwrap();
        }
        let library = Self {
            settings: Settings {
                source,
                state_dir: root.join("state"),
                machine: "test".into(),
                ..Default::default()
            },
            root,
            _temp: temp,
        };
        library.add("review", SKILL);
        for index in 0..skills {
            let name = format!("skill-{index}");
            library.add(
                &name,
                &format!("---\nname: {name}\ndescription: Skill {index}\n---\nBody {index}.\n"),
            );
        }
        library
    }

    fn add(&self, name: &str, markdown: &str) {
        let skill = self.settings.source.join("skills").join(name);
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), markdown).unwrap();
        fs::write(skill.join("skill.toml"), "").unwrap();
    }

    fn preview(&self, markdown: &str) -> anyhow::Result<Vec<ratatoskr_core::PreviewFile>> {
        ratatoskr_core::preview_skill(
            &self.settings,
            None,
            "review",
            markdown,
            "",
            "test",
            "codex",
        )
    }

    fn save(&self, markdown: &str) -> anyhow::Result<()> {
        let current = ratatoskr_core::read_skill(&self.settings, None, "review")?;
        ratatoskr_core::save_skill_draft(
            &self.settings,
            None,
            "review",
            markdown,
            &current.metadata,
            &current.markdown_hash,
            &current.metadata_hash,
        )
    }

    fn state(&self, file: &str) -> PathBuf {
        self.settings.state_dir.join("test").join(file)
    }
}

fn every_file(root: &Path) -> Vec<PathBuf> {
    let mut files = vec![];
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    files
}

/// Atomic writes replace the file, so an unchanged identity proves no rewrite happened.
#[cfg(unix)]
fn identity(path: &Path) -> (u64, std::time::SystemTime) {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path).unwrap();
    (metadata.ino(), metadata.modified().unwrap())
}

#[test]
fn other_skills_cannot_affect_previewing_or_saving_this_one() {
    let library = Library::new(2);
    // Reading this skill at all would fail, so success proves it is never rendered.
    let broken = library.settings.source.join("skills/broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("SKILL.md"), "not a skill").unwrap();
    fs::write(broken.join("skill.toml"), "not = valid = toml").unwrap();
    let draft = SKILL.replace("Review changes.\n", "Review carefully.\n");
    let files = library.preview(&draft).unwrap();
    assert!(
        files[0]
            .text
            .as_ref()
            .unwrap()
            .contains("Review carefully.")
    );
    library.save(&draft).unwrap();
    assert_eq!(
        ratatoskr_core::read_skill(&library.settings, None, "review")
            .unwrap()
            .markdown,
        draft
    );
}

#[test]
fn saving_still_validates_every_machine_and_harness() {
    let library = Library::new(0);
    fs::write(
        library.settings.source.join("machines/other.toml"),
        "os='macos'\nmode='staging'\nharnesses=['codex']\nonly_here='x'\n",
    )
    .unwrap();
    let invalid = "---\nname: review\ndescription: Review\n---\n<!-- only machine=other -->{{af:missing}}<!-- /only -->Body.\n";
    let error = format!("{:#}", library.save(invalid).unwrap_err());
    assert!(error.contains("other/codex"), "{error}");
    assert_eq!(
        ratatoskr_core::read_skill(&library.settings, None, "review")
            .unwrap()
            .markdown,
        SKILL
    );
}

#[test]
fn a_draft_preview_and_save_leave_no_files_behind_in_the_source() {
    let library = Library::new(3);
    let before = every_file(&library.settings.source);
    library.preview(SKILL).unwrap();
    assert_eq!(every_file(&library.settings.source), before);
    assert!(!library.settings.state_dir.exists());
}

#[test]
fn preview_cost_does_not_grow_with_the_library() {
    let library = Library::new(600);
    let started = Instant::now();
    // One sentence of typing. The old preview took about a second per call on a smaller library.
    for pause in 0..200 {
        library
            .preview(&SKILL.replace("Review changes.", &format!("Draft {pause}.")))
            .unwrap();
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(10),
        "200 previews took {elapsed:?}"
    );
}

#[test]
fn saving_a_skill_does_not_render_the_library() {
    let library = Library::new(600);
    let started = Instant::now();
    for attempt in 0..5 {
        library
            .save(&SKILL.replace("Review changes.", &format!("Revision {attempt}.")))
            .unwrap();
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(10),
        "5 saves took {elapsed:?}"
    );
}

#[cfg(unix)]
#[test]
fn applying_with_nothing_to_do_rewrites_nothing() {
    let library = Library::new(3);
    apply(&library.settings).unwrap();
    let manifest = library.state("manifest.json");
    let outputs = every_file(&library.settings.state_dir.join("test/staging"));
    assert!(!outputs.is_empty());
    let snapshot = |paths: &[PathBuf]| paths.iter().map(|p| identity(p)).collect::<Vec<_>>();
    let manifest_before = identity(&manifest);
    let outputs_before = snapshot(&outputs);
    let report = apply(&library.settings).unwrap();
    assert!(report.changes.is_empty());
    let applied = report.applied.unwrap();
    assert_eq!((applied.added, applied.updated, applied.removed), (0, 0, 0));
    assert_eq!(identity(&manifest), manifest_before);
    assert_eq!(snapshot(&outputs), outputs_before);
}

#[test]
fn applying_a_real_change_still_reports_the_final_state() {
    let library = Library::new(1);
    apply(&library.settings).unwrap();
    library.add(
        "review",
        &SKILL.replace("Review changes.\n", "Review again.\n"),
    );
    let report = apply(&library.settings).unwrap();
    assert_eq!(report.applied.unwrap().updated, 2, "once per harness");
    assert!(
        report.changes.is_empty(),
        "the report is the post-apply plan"
    );
}

#[test]
fn reading_a_project_skill_needs_no_git_process() {
    let library = Library::new(0);
    // Not a Git repository: project_status would report an error for it, while every editor read
    // only needs the marker. Spawning Git here cost four processes per read per project.
    let project = library.root.join("project");
    fs::create_dir_all(project.join("agent-skills/skills/local")).unwrap();
    fs::write(project.join(".ratatoskr.toml"), "source = 'agent-skills'\n").unwrap();
    fs::write(
        project.join("agent-skills/skills/local/SKILL.md"),
        "---\nname: local\ndescription: Local\n---\nBody.\n",
    )
    .unwrap();
    fs::write(project.join("agent-skills/skills/local/skill.toml"), "").unwrap();
    let mut settings = library.settings.clone();
    settings.projects.push(project.clone());
    let document = ratatoskr_core::read_skill(&settings, Some(&project), "local").unwrap();
    assert!(document.markdown.contains("Body."));
    assert!(ratatoskr_core::project_status(&project).error.is_some());
}

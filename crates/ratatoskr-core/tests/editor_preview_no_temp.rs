//! This binary holds one test because it changes the process-wide temporary directory.
use std::{fs, path::PathBuf};

/// A preview runs on every pause in typing. Pointing the temporary directory at nothing proves it
/// never builds a scratch copy, whichever way a future change might try to.
#[test]
fn draft_preview_never_touches_the_temporary_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = root.join("content");
    fs::create_dir_all(source.join("machines")).unwrap();
    fs::create_dir_all(source.join("skills/review")).unwrap();
    fs::write(
        source.join("harnesses.toml"),
        "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\nroots={linux='/tmp/x',macos='/tmp/x',windows='C:/x'}\n",
    )
    .unwrap();
    fs::write(
        source.join("machines/test.toml"),
        "os='macos'\nmode='staging'\nharnesses=['codex']\n",
    )
    .unwrap();
    let markdown = "---\nname: review\ndescription: Review\n---\nSaved.\n";
    fs::write(source.join("skills/review/SKILL.md"), markdown).unwrap();
    fs::write(source.join("skills/review/skill.toml"), "").unwrap();
    let settings = ratatoskr_core::Settings {
        source,
        state_dir: root.join("state"),
        machine: "test".into(),
        ..Default::default()
    };
    let missing: PathBuf = root.join("no-such-temporary-directory");
    // SAFETY: this is the only test in this binary, so no other thread reads the environment.
    unsafe {
        for name in ["TMPDIR", "TEMP", "TMP"] {
            std::env::set_var(name, &missing);
        }
    }
    assert!(tempfile::tempdir().is_err(), "the guard must be effective");
    let files = ratatoskr_core::preview_skill(
        &settings,
        None,
        "review",
        "---\nname: review\ndescription: Review\n---\nDraft.\n",
        "",
        "test",
        "codex",
    )
    .unwrap();
    assert!(files[0].text.as_ref().unwrap().contains("Draft."));
    assert!(!missing.exists());
    assert!(
        !settings.state_dir.exists(),
        "a preview must not write state"
    );
}

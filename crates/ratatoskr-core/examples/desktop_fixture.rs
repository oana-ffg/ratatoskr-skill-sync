//! Public, disposable desktop fixture/IPC oracle. Never reads the user's workspace.
use ratatoskr_core as core;
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read},
};
fn main() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let source = root.join("content");
    fs::create_dir_all(source.join("machines"))?;
    fs::create_dir_all(source.join("skills/recordings"))?;
    fs::write(
        source.join("harnesses.toml"),
        "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\nproject_root='.agents'\n[codex.roots]\nmacos='{{af:home}}/.agents'\nwindows='{{af:home}}/.agents'\nlinux='{{af:home}}/.agents'\n[claude]\nskills_dir='skills'\nglobal_file='CLAUDE.md'\nproject_root='.claude'\n[claude.roots]\nmacos='{{af:home}}/.claude'\nwindows='{{af:home}}/.claude'\nlinux='{{af:home}}/.claude'\n",
    )?;
    let machine = "os='macos'\nmode='staging'\nhome='/fixture-home'\nharnesses=['codex','claude']\ncapabilities=['transcription','gopass']\n[paths]\nrecordings_archive='/archive/recordings'\n";
    for name in ["brunnr", "skirnir", "loki"] {
        let profile = if name == "loki" {
            "os='windows'\nmode='staging'\nhome='C:\\Users\\Oana'\nharnesses=['codex','claude']\ncapabilities=['transcription']\n[paths]\nrecordings_archive='C:\\Users\\Oana\\Recordings'\n"
        } else {
            machine
        };
        fs::write(source.join(format!("machines/{name}.toml")), profile)?;
    }
    let md = "---\nname: recordings\ndescription: Process meeting recordings into a searchable archive.\n---\n# Meeting recordings\n\nKeep the original recording in {{af:paths.recordings_archive}}.\n\n## Steps\n1. Copy the recording and **verify its checksum**.\n2. Transcribe the audio.\n\n<!-- only harness=codex -->\nUse the automation tools to manage scheduled tasks.\n<!-- /only -->\n\n<!-- only os=windows -->\nUse Get-FileHash to verify the copied recording.\n<!-- /only -->\n\n<!-- only os=linux -->\nUse sha256sum to verify the copied recording.\n<!-- /only -->\n\n3. Review the transcript before archiving.\n";
    let meta = "# Preserve this shared targeting\n[target]\nharnesses = ['claude', 'codex']\ncapabilities = ['transcription']\n";
    fs::write(source.join("skills/recordings/SKILL.md"), md)?;
    fs::write(source.join("skills/recordings/skill.toml"), meta)?;
    fs::create_dir_all(source.join("skills/recordings/scripts"))?;
    fs::write(
        source.join("skills/recordings/scripts/hash.sh"),
        "shasum -a 256 \"$1\"\n",
    )?;
    fs::create_dir_all(source.join("skills/recordings/variants/os=windows/scripts"))?;
    fs::write(
        source.join("skills/recordings/variants/os=windows/scripts/hash.ps1"),
        "Get-FileHash $args[0]\n",
    )?;
    for (name, description, body, metadata) in [
        (
            "review-code",
            "Review changes for correctness and maintainability.",
            "Check changed behavior, failure paths and test coverage.",
            "[target]\nharnesses=['codex']\n",
        ),
        (
            "architecture-notes",
            "Record architectural decisions and their tradeoffs.",
            "Explain the problem, constraints and decision.",
            "",
        ),
        (
            "release-checklist",
            "Verify a release before publishing it.",
            "Run checks and inspect the packaged application.",
            "[target]\nos=['windows']\n",
        ),
        (
            "writing-style",
            "Keep technical writing clear and concise.",
            "Lead with the concrete outcome.",
            "enabled=false\n",
        ),
    ] {
        let dir = source.join("skills").join(name);
        fs::create_dir_all(&dir)?;
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n{body}\n"),
        )?;
        fs::write(dir.join("skill.toml"), metadata)?;
    }
    let settings = core::Settings {
        source,
        machine: "brunnr".into(),
        state_dir: root.join("state"),
        ..Default::default()
    };
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let request: Value = if input.is_empty() {
        json!({"cmd":"fixture"})
    } else {
        serde_json::from_str(&input)?
    };
    let args = &request["args"];
    let text = args["markdown"].as_str().unwrap_or(md);
    let metadata = args["metadata"].as_str().unwrap_or(meta);
    let m = args["machine"].as_str().unwrap_or("brunnr");
    let h = args["harness"].as_str().unwrap_or("codex");
    let result = match request["cmd"].as_str().unwrap_or("fixture") {
        "preview_install_path" => serde_json::to_value(core::preview_install_path(
            &settings,
            None,
            "recordings",
            m,
            h,
        )?)?,
        "different_preview_lines" => serde_json::to_value(core::different_preview_lines(
            args["text"].as_str().unwrap_or(""),
            &serde_json::from_value::<Vec<String>>(args["versions"].clone())?,
        ))?,
        "editor_view" => serde_json::to_value(core::editor_view(text, metadata)?)?,
        "analyze_skill" => {
            serde_json::to_value(core::analyze_skill(&settings, "recordings", text, m, h)?)?
        }
        "preview_skill" => serde_json::to_value(core::preview_skill(
            &settings,
            None,
            "recordings",
            text,
            metadata,
            m,
            h,
        )?)?,
        "edit_skill_draft" => serde_json::to_value(core::edit_draft(
            text,
            metadata,
            serde_json::from_value(args["edit"].clone())?,
        )?)?,
        "read_structured_source" => serde_json::to_value(core::structured_source(
            &settings,
            args["file"].as_str().unwrap_or("harnesses.toml"),
        )?)?,
        "fixture" => {
            core::apply(&settings)?;
            let installed = settings
                .state_dir
                .join("brunnr/staging/codex/skills/recordings/SKILL.md");
            let original = fs::read_to_string(&installed)?;
            fs::write(
                &installed,
                original.replace(
                    "Review the transcript",
                    "Carefully review the complete transcript",
                ),
            )?;
            let claude = settings
                .state_dir
                .join("brunnr/staging/claude/skills/recordings/SKILL.md");
            fs::write(
                &claude,
                fs::read_to_string(&claude)?
                    .replace("2. Transcribe the audio.", "2. Transcribe audio."),
            )?;
            // Adopt this version as the previously managed copy; the source now has a ready update.
            let preview =
                core::preview_resolution(&settings, &claude, core::ConflictAction::Adopt)?;
            core::resolve_conflict(&settings, &serde_json::to_string(&preview)?)?;
            let mut plan = serde_json::to_value(core::plan(&settings)?)?;
            plan["projects"] = json!([{"root":"/projects/audio-tools","changed_files":2,"outgoing_commits":1,"upstream":"origin/main","error":null,"source":"/projects/audio-tools/.ratatoskr"},{"root":"/projects/website","changed_files":0,"outgoing_commits":0,"upstream":"origin/main","error":null,"source":"/projects/website/.ratatoskr"}]);
            let skills = core::list_skills(&settings)?;
            json!({"workspace":{"plan":plan,"skills":skills,"helper":"/Applications/Ratatoskr.app/Contents/MacOS/ratatoskr","helper_error":null,"jobs":{"last_run":{"started_at":1792173480_u64,"finished_at":1792173490_u64,"error":null},"current_error":null,"helper_update_error":null,"desktop_update_error":null,"log":"18:35:00  Checking content repository\n18:35:01  Source unchanged\n18:35:02  Rendered 5 skills for 2 agents\n18:35:03  Sync completed successfully"},"presets":core::agent_presets(),"preview_targets":core::preview_targets(&settings)?,"source_files":core::source_files(&settings)?,"updates":{"automatic":true,"time":"02:00","override_active":false,"endpoint":"https://github.com/oana-ffg/ratatoskr-skill-sync/releases/latest/download/latest.json"},"legacy":[]},"document":core::read_skill(&settings,None,"recordings")?,"info":core::editor_view(md,meta)?,"analysis":core::analyze_skill(&settings,"recordings",md,"brunnr","codex")?,"values":core::value_options(&settings,"brunnr","codex")?,"resources":core::skill_resources(&settings,None,"recordings")?,"statuses":core::skill_target_statuses(&settings)?,"local_settings":settings})
        }
        other => anyhow::bail!("Unsupported fixture command: {other}"),
    };
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

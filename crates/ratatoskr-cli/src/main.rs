use anyhow::Result;
use clap::{Parser, Subcommand};
use ratatoskr_core as core;
use ratatoskr_core::{Plan, add_project, apply, default_config_path, initialize, load, plan, sync};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ratatoskr", version, about = "Ratatoskr Skill Sync")]
struct Cli {
    /// Local configuration file (or RATATOSKR_CONFIG). Custom init paths use <config filename>.state/.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Connect a local skill-content checkout and machine profile. Does not apply.
    Init {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        machine: String,
        /// Absolute state directory. Defaults to <config filename>.state/ beside a custom config; otherwise platform data.
        #[arg(long, value_name = "DIR")]
        state_dir: Option<PathBuf>,
        #[arg(long)]
        create: bool,
        #[arg(long, conflicts_with = "create")]
        remote: Option<String>,
    },
    /// Render every machine profile without configuration or filesystem writes.
    Validate { source: PathBuf },
    /// Validate and preview all output changes, including text diffs.
    Plan,
    /// Check generated output and monitored projects. Exit 1 for pending work.
    Status,
    /// Apply only if every destination passes the ownership checks.
    Apply,
    /// Preview or explicitly apply a single conflict resolution.
    Conflict {
        #[command(subcommand)]
        action: ConflictCommand,
    },
    /// Last background run, notification-ledger errors and bounded logs.
    JobStatus,
    /// List verified presets or preview an agent definition from a JSON input file.
    Agent {
        #[command(subcommand)]
        action: AgentCommand,
    },
    /// Fast-forward a clean content checkout, then apply without the desktop UI.
    Sync,
    /// Back up and explicitly adopt existing matching destination directories.
    Adopt,
    /// Restore a verified pre-adoption backup without overwriting hand edits.
    Restore { backup: PathBuf },
    /// Create, edit, disable or archive source skills.
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// Preview, install or remove the per-user OS schedule.
    Schedule {
        #[command(subcommand)]
        action: ScheduleAction,
    },
    /// Check for a newer signed helper, optionally installing it.
    Update {
        #[arg(long)]
        install: bool,
    },
    /// Register or unregister a project for skill-source Git monitoring.
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
}

#[derive(Subcommand)]
enum ConflictCommand {
    /// Read the mapped source, including a selected variant.
    Source { path: PathBuf },
    /// Validate and preview a manual source edit when copy-back is ambiguous.
    EditSource {
        path: PathBuf,
        #[arg(long)]
        input: PathBuf,
    },
    Preview {
        path: PathBuf,
        #[arg(long, value_parser = ["copy-back", "reinstall", "adopt"])]
        action: String,
    },
    /// Apply the exact JSON preview saved from conflict preview.
    Resolve {
        #[arg(long)]
        preview: PathBuf,
    },
}

#[derive(Subcommand)]
enum AgentCommand {
    Presets,
    Preview {
        #[arg(long)]
        input: PathBuf,
    },
    Add {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        expected_hash: String,
    },
}

#[derive(Subcommand)]
enum SkillAction {
    Analyze {
        name: String,
        #[arg(long)]
        machine: String,
        #[arg(long)]
        harness: String,
        #[arg(long)]
        input: Option<PathBuf>,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    Preview {
        name: String,
        #[arg(long)]
        machine: String,
        #[arg(long)]
        harness: String,
        #[arg(long)]
        markdown: Option<PathBuf>,
        #[arg(long)]
        metadata: Option<PathBuf>,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    List,
    Read {
        name: String,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    Create {
        name: String,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    Write {
        name: String,
        #[arg(long)]
        file: String,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        expected_hash: String,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    Disable {
        name: String,
        #[arg(long)]
        enable: bool,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    Archive {
        name: String,
        #[arg(long)]
        project: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ScheduleAction {
    Plan {
        #[arg(long)]
        helper: Option<PathBuf>,
        #[arg(long, default_value_t = 60)]
        seconds: u32,
    },
    Install {
        #[arg(long)]
        helper: Option<PathBuf>,
        #[arg(long, default_value_t = 60)]
        seconds: u32,
    },
    Remove,
}

#[derive(Subcommand)]
enum ProjectAction {
    /// Source is relative to the repository root; it must already exist.
    Add {
        root: PathBuf,
        #[arg(long, default_value = "agent-skills")]
        source: String,
    },
    /// Removes only the local registry entry. Keeps the project's marker and files.
    Remove { root: PathBuf },
}

fn show(plan: &Plan, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
        return Ok(());
    }
    println!(
        "{} | {} | {} skill installations across {} targets",
        plan.machine, plan.mode, plan.skills, plan.targets
    );
    for change in &plan.changes {
        println!("{} {}", change.action, change.path.display());
        if let Some(diff) = &change.diff {
            print!("{diff}");
        }
    }
    for conflict in &plan.conflicts {
        println!("CONFLICT: {conflict}");
    }
    for project in &plan.projects {
        println!(
            "{}: {} changed skill files, {} outgoing skill commits{}",
            project.root.display(),
            project.changed_files,
            project
                .outgoing_commits
                .map_or("unknown (no upstream)".into(), |n| n.to_string()),
            project
                .error
                .as_ref()
                .map_or(String::new(), |e| format!(", ERROR: {e}"))
        );
    }
    if plan.changes.is_empty() && plan.conflicts.is_empty() {
        println!("Generated files are up to date.");
    }
    Ok(())
}

fn counted(count: usize, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

fn run() -> Result<i32> {
    let cli = Cli::parse();
    if let Action::Validate { source } = &cli.command {
        let report = core::validate_source(source)?;
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            for profile in &report.profiles {
                let skills: usize = profile.harnesses.iter().map(|h| h.skills_rendered).sum();
                println!(
                    "{}: {} across {} rendered; {}",
                    profile.machine,
                    counted(skills, "skill installation", "skill installations"),
                    counted(profile.harnesses.len(), "harness", "harnesses"),
                    counted(
                        profile.problem_occurrences,
                        "problem occurrence",
                        "problem occurrences"
                    )
                );
                for harness in &profile.harnesses {
                    println!(
                        "  {}: {}, {} skipped, {}; global instructions {}",
                        harness.harness,
                        counted(harness.skills_rendered, "skill", "skills"),
                        harness.skills_skipped,
                        counted(harness.files_rendered, "file", "files"),
                        if harness.global_instructions_rendered {
                            "rendered"
                        } else {
                            "not rendered"
                        }
                    );
                }
            }
            for problem in &report.problems {
                let contexts = problem
                    .contexts
                    .iter()
                    .map(|context| {
                        let mut parts = vec![context.machine.as_str()];
                        parts.extend(context.harness.as_deref());
                        parts.extend(context.skill.as_deref());
                        parts.join("/")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("ERROR {}: {} [{}]", problem.path, problem.message, contexts);
            }
            println!(
                "{}; {}; {}",
                counted(report.profiles.len(), "profile", "profiles"),
                counted(
                    report.problems.len(),
                    "distinct problem",
                    "distinct problems"
                ),
                counted(
                    report.problem_occurrences,
                    "problem occurrence",
                    "problem occurrences"
                )
            );
        }
        return Ok(i32::from(!report.problems.is_empty()));
    }
    let path = cli.config.map(Ok).unwrap_or_else(default_config_path)?;
    match cli.command {
        Action::Init {
            source,
            machine,
            state_dir,
            create,
            remote,
        } => {
            let settings = if create {
                core::create_content(&path, &source, &machine, state_dir.as_deref())?
            } else if let Some(remote) = remote {
                core::clone_content(&path, &remote, &source, &machine, state_dir.as_deref())?
            } else {
                initialize(&path, &source, &machine, state_dir.as_deref())?
            };
            show(&plan(&settings)?, cli.json)?;
            eprintln!("Saved {}. No skills were applied.", path.display());
        }
        Action::Project { action } => match action {
            ProjectAction::Add { root, source } => println!(
                "{}",
                serde_json::to_string_pretty(&add_project(&path, &root, &source)?)?
            ),
            ProjectAction::Remove { root } => {
                core::remove_project(&path, &root)?;
                println!("Removed project from local registry; project files preserved.");
            }
        },
        Action::Conflict { action } => {
            let settings = load(&path)?;
            match action {
                ConflictCommand::Source { path } => println!(
                    "{}",
                    serde_json::to_string_pretty(&core::read_conflict_source(&settings, &path)?)?
                ),
                ConflictCommand::EditSource { path, input } => println!(
                    "{}",
                    serde_json::to_string_pretty(&core::preview_source_edit(
                        &settings,
                        &path,
                        &std::fs::read_to_string(input)?
                    )?)?
                ),
                ConflictCommand::Preview { path, action } => println!(
                    "{}",
                    serde_json::to_string_pretty(&core::preview_resolution(
                        &settings,
                        &path,
                        serde_json::from_value(serde_json::Value::String(action))?
                    )?)?
                ),
                ConflictCommand::Resolve { preview } => println!(
                    "Verified backup: {}",
                    core::resolve_conflict(&settings, &std::fs::read_to_string(preview)?)?
                        .display()
                ),
            }
        }
        Action::JobStatus => println!(
            "{}",
            serde_json::to_string_pretty(&core::job_status(&load(&path)?)?)?
        ),
        Action::Agent { action } => match action {
            AgentCommand::Presets => {
                println!("{}", serde_json::to_string_pretty(&core::agent_presets())?)
            }
            AgentCommand::Preview { input } => {
                let target = serde_json::from_str(&std::fs::read_to_string(input)?)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&core::preview_agent_target(
                        &load(&path)?,
                        &target
                    )?)?
                );
            }
            AgentCommand::Add {
                input,
                expected_hash,
            } => {
                let target = serde_json::from_str(&std::fs::read_to_string(input)?)?;
                let settings = load(&path)?;
                core::add_agent_target(&settings, &target, &expected_hash)?;
                println!(
                    "Agent definition saved. Enable its tag in the intended machine profiles."
                );
            }
        },
        Action::Adopt => {
            println!("Verified backup: {}", core::adopt(&load(&path)?)?.display());
        }
        Action::Restore { backup } => {
            show(&core::restore_backup(&load(&path)?, &backup)?, cli.json)?
        }
        Action::Skill { action } => {
            let settings = load(&path)?;
            match action {
                SkillAction::Analyze {
                    name,
                    machine,
                    harness,
                    input,
                    project,
                } => {
                    let markdown = if let Some(path) = input {
                        std::fs::read_to_string(path)?
                    } else {
                        core::read_skill(&settings, project.as_deref(), &name)?.markdown
                    };
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&core::analyze_skill(
                            &settings, &name, &markdown, &machine, &harness
                        )?)?
                    );
                }
                SkillAction::Preview {
                    name,
                    machine,
                    harness,
                    markdown,
                    metadata,
                    project,
                } => {
                    let document = core::read_skill(&settings, project.as_deref(), &name)?;
                    let markdown = markdown
                        .map(std::fs::read_to_string)
                        .transpose()?
                        .unwrap_or(document.markdown);
                    let metadata = metadata
                        .map(std::fs::read_to_string)
                        .transpose()?
                        .unwrap_or(document.metadata);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&core::preview_skill(
                            &settings,
                            project.as_deref(),
                            &name,
                            &markdown,
                            &metadata,
                            &machine,
                            &harness
                        )?)?
                    );
                }
                SkillAction::List => println!(
                    "{}",
                    serde_json::to_string_pretty(&core::list_skills(&settings)?)?
                ),
                SkillAction::Read { name, project } => println!(
                    "{}",
                    serde_json::to_string_pretty(&core::read_skill(
                        &settings,
                        project.as_deref(),
                        &name
                    )?)?
                ),
                SkillAction::Create {
                    name,
                    input,
                    project,
                } => core::create_skill(
                    &settings,
                    project.as_deref(),
                    &name,
                    &std::fs::read_to_string(input)?,
                )?,
                SkillAction::Write {
                    name,
                    file,
                    input,
                    expected_hash,
                    project,
                } => core::write_skill_file(
                    &settings,
                    project.as_deref(),
                    &name,
                    &file,
                    &std::fs::read_to_string(input)?,
                    &expected_hash,
                )?,
                SkillAction::Disable {
                    name,
                    enable,
                    project,
                } => core::disable_skill(&settings, project.as_deref(), &name, !enable)?,
                SkillAction::Archive { name, project } => println!(
                    "Archived: {}",
                    core::archive_skill(&settings, project.as_deref(), &name)?.display()
                ),
            }
        }
        Action::Schedule { action } => {
            #[cfg(windows)]
            anyhow::ensure!(
                matches!(action, ScheduleAction::Remove),
                "Windows uses tray sync. Open Ratatoskr Skill Sync and enable Open at login in Settings; schedule install is disabled."
            );
            let settings = load(&path)?;
            match action {
                ScheduleAction::Plan { helper, seconds } => println!(
                    "{}",
                    core::schedule_plan(
                        &settings,
                        &path,
                        &helper.map(Ok).unwrap_or_else(|| core::detect_helper(
                            &std::env::current_exe()?,
                            std::env::var_os("PATH").as_deref()
                        ))?,
                        seconds
                    )?
                    .definition
                ),
                ScheduleAction::Install { helper, seconds } => core::install_schedule(
                    &settings,
                    &core::schedule_plan(
                        &settings,
                        &path,
                        &helper.map(Ok).unwrap_or_else(|| {
                            core::detect_helper(
                                &std::env::current_exe()?,
                                std::env::var_os("PATH").as_deref(),
                            )
                        })?,
                        seconds,
                    )?,
                )?,
                ScheduleAction::Remove => core::remove_schedule(&settings)?,
            }
        }
        Action::Update { install } => {
            let settings = load(&path)?;
            let update = &settings.updates;
            let result: anyhow::Result<()> = (|| {
                if let Some(release) = core::updates::check(update, env!("CARGO_PKG_VERSION"))? {
                    println!("Update available: {}", release.version);
                    if install {
                        core::updates::install(update, &release)?;
                        println!("Signed update installed.");
                    }
                } else {
                    println!("No newer release available.");
                }
                Ok(())
            })();
            let error = result
                .as_ref()
                .err()
                .map(|e| format!("Helper update: {e:#}"));
            if let Err(notification) = core::updates::report_outcome(
                &settings,
                error.as_deref(),
                core::updates::UpdateKind::Helper,
            ) {
                anyhow::bail!(
                    "{}; alert delivery failed: {notification:#}",
                    error.as_deref().unwrap_or("Helper update completed")
                );
            }
            result?;
        }
        command => {
            let settings = load(&path)?;
            let report = match command {
                Action::Apply => apply(&settings)?,
                Action::Sync => sync(&settings)?,
                _ => plan(&settings)?,
            };
            show(&report, cli.json)?;
            if matches!(command, Action::Status)
                && (!report.changes.is_empty()
                    || !report.conflicts.is_empty()
                    || report.projects.iter().any(|p| p.needs_attention()))
            {
                return Ok(1);
            }
            if !report.conflicts.is_empty() {
                return Ok(1);
            }
        }
    }
    Ok(0)
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("Ratatoskr: {error:#}");
            std::process::exit(2);
        }
    }
}

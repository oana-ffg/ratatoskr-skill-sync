use anyhow::Result;
use clap::{Parser, Subcommand};
use ratatoskr_core as core;
use ratatoskr_core::{Plan, add_project, apply, default_config_path, initialize, load, plan, sync};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ratatoskr", version, about = "Ratatoskr Skill Sync")]
struct Cli {
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
        #[arg(long)]
        create: bool,
        #[arg(long, conflicts_with = "create")]
        remote: Option<String>,
    },
    /// Validate and preview all output changes, including text diffs.
    Plan,
    /// Check generated output and monitored projects. Exit 1 for pending work.
    Status,
    /// Apply only if every destination passes the ownership checks.
    Apply,
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
enum SkillAction {
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
        helper: PathBuf,
        #[arg(long, default_value_t = 60)]
        seconds: u32,
    },
    Install {
        #[arg(long)]
        helper: PathBuf,
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

fn run() -> Result<i32> {
    let cli = Cli::parse();
    let path = cli.config.map(Ok).unwrap_or_else(default_config_path)?;
    match cli.command {
        Action::Init {
            source,
            machine,
            create,
            remote,
        } => {
            let settings = if create {
                core::create_content(&path, &source, &machine)?
            } else if let Some(remote) = remote {
                core::clone_content(&path, &remote, &source, &machine)?
            } else {
                initialize(&path, &source, &machine)?
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
        Action::Adopt => {
            println!("Verified backup: {}", core::adopt(&load(&path)?)?.display());
        }
        Action::Restore { backup } => {
            show(&core::restore_backup(&load(&path)?, &backup)?, cli.json)?
        }
        Action::Skill { action } => {
            let settings = load(&path)?;
            match action {
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
            let settings = load(&path)?;
            match action {
                ScheduleAction::Plan { helper, seconds } => println!(
                    "{}",
                    core::schedule_plan(&settings, &path, &helper, seconds)?.definition
                ),
                ScheduleAction::Install { helper, seconds } => core::install_schedule(
                    &settings,
                    &core::schedule_plan(&settings, &path, &helper, seconds)?,
                )?,
                ScheduleAction::Remove => core::remove_schedule(&settings)?,
            }
        }
        Action::Update { install } => {
            let settings = load(&path)?;
            let update = settings.updates.as_ref().ok_or_else(|| {
                anyhow::anyhow!("Configure an HTTPS update endpoint and trusted public key first")
            })?;
            if let Some(release) = core::updates::check(update, env!("CARGO_PKG_VERSION"))? {
                println!("Update available: {}", release.version);
                if install {
                    core::updates::install(update, &release)?;
                    println!("Signed update installed.");
                }
            } else {
                println!("No newer release available.");
            }
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

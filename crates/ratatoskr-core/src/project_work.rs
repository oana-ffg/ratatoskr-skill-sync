//! Project skill work in progress is normal; only work left pending past a grace
//! period is a problem. Alert text names the pending items without counts so the
//! alert ledger does not repeat it every time another file changes.

use crate::{
    Settings,
    config::{atomic_write, no_symlinks},
    git::ProjectStatus,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const GRACE_SECS: u64 = 60 * 60;
const LEDGER: &str = "project-work.json";

/// When each currently pending item of each project was first seen.
#[derive(Debug, Default, Deserialize, PartialEq, Serialize)]
struct Ledger {
    first_seen: BTreeMap<PathBuf, BTreeMap<String, u64>>,
}

/// Caller holds sync.lock, which serializes ledger updates.
pub(crate) fn check(settings: &Settings, projects: &[ProjectStatus]) -> Result<()> {
    let path = settings.state_dir.join(&settings.machine).join(LEDGER);
    no_symlinks(&path)?;
    let previous = if path.try_exists()? {
        serde_json::from_slice(&fs::read(&path)?)?
    } else {
        Ledger::default()
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let (ledger, problems) = track(&previous, projects, now);
    if ledger != previous {
        atomic_write(&path, &serde_json::to_vec(&ledger)?)?;
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(crate::tray::Problem::message(
        "Project work pending",
        format!("Outputs synced; {}", problems.join("; ")),
    )
    .into())
}

/// Inspection errors are reported at once; pending work once it outlives the grace period.
fn track(previous: &Ledger, projects: &[ProjectStatus], now: u64) -> (Ledger, Vec<String>) {
    let mut ledger = Ledger::default();
    let mut failed = Vec::new();
    let mut overdue = Vec::new();
    for project in projects {
        let root = project.root.display();
        if let Some(error) = &project.error {
            failed.push(format!("{root} ({error})"));
            continue;
        }
        let seen = previous.first_seen.get(&project.root);
        let items: BTreeMap<_, _> = pending_items(project)
            .into_iter()
            .map(|item| {
                let since = seen.and_then(|s| s.get(&item)).copied().unwrap_or(now);
                (item, since)
            })
            .collect();
        let late: Vec<_> = items
            .iter()
            .filter(|(_, since)| now.saturating_sub(**since) >= GRACE_SECS)
            .map(|(item, _)| item.as_str())
            .collect();
        if !late.is_empty() {
            overdue.push(format!("{root} ({})", late.join(", ")));
        }
        if !items.is_empty() {
            ledger.first_seen.insert(project.root.clone(), items);
        }
    }
    let mut problems = Vec::new();
    if !failed.is_empty() {
        problems.push(format!("project inspection failed: {}", failed.join("; ")));
    }
    if !overdue.is_empty() {
        problems.push(format!(
            "project skill work pending for over an hour: {}",
            overdue.join("; ")
        ));
    }
    (ledger, problems)
}

fn pending_items(project: &ProjectStatus) -> BTreeSet<String> {
    let skills = project
        .source
        .as_ref()
        .and_then(|source| source.strip_prefix(&project.root).ok())
        .map(|source| source.join("skills"));
    let mut items: BTreeSet<_> = project
        .changed_paths
        .iter()
        .map(|path| {
            match skills
                .as_ref()
                .and_then(|skills| path.strip_prefix(skills).ok())
                .and_then(|skill| skill.components().next())
            {
                Some(Component::Normal(name)) => format!("skill {}", name.to_string_lossy()),
                _ => "other skill source files".into(),
            }
        })
        .collect();
    match project.outgoing_commits {
        Some(0) => {}
        Some(_) => {
            items.insert("unpushed commits".into());
        }
        None => {
            items.insert("no upstream branch".into());
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(changed: &[&str], outgoing: Option<usize>) -> ProjectStatus {
        ProjectStatus {
            root: "/work/brain".into(),
            source: Some("/work/brain/agent-skills".into()),
            changed_files: changed.len(),
            outgoing_commits: outgoing,
            upstream: None,
            error: None,
            changed_paths: changed.iter().map(PathBuf::from).collect(),
        }
    }

    #[test]
    fn pending_work_alerts_per_item_only_after_the_grace_period() {
        let start = 1_000_000;
        let editing = project(&["agent-skills/skills/notes/SKILL.md"], Some(0));
        let (ledger, problems) = track(&Ledger::default(), std::slice::from_ref(&editing), start);
        assert!(problems.is_empty());

        // More files in the same skill neither restart its clock nor change the alert.
        let more = project(
            &[
                "agent-skills/skills/notes/SKILL.md",
                "agent-skills/skills/notes/scripts/run.js",
                "agent-skills/skills/mail/SKILL.md",
            ],
            Some(0),
        );
        let (ledger, problems) =
            track(&ledger, std::slice::from_ref(&more), start + GRACE_SECS - 1);
        assert!(problems.is_empty());
        let (ledger, problems) = track(&ledger, std::slice::from_ref(&more), start + GRACE_SECS);
        assert_eq!(
            problems,
            ["project skill work pending for over an hour: /work/brain (skill notes)"]
        );
        let (_, again) = track(&ledger, &[editing], start + GRACE_SECS + 10);
        assert_eq!(again, problems);

        // Finishing the work resets its clock.
        let (ledger, problems) = track(&ledger, &[project(&[], Some(0))], start + GRACE_SECS);
        assert!(problems.is_empty() && ledger.first_seen.is_empty());
        let (_, problems) = track(&ledger, &[more], start + 2 * GRACE_SECS);
        assert!(problems.is_empty());
    }

    #[test]
    fn unpushed_work_settings_and_errors_are_named_without_counts() {
        let start = 1_000_000;
        let work = project(&["agent-skills/harnesses.toml", ".ratatoskr.toml"], Some(3));
        let (ledger, _) = track(&Ledger::default(), std::slice::from_ref(&work), start);
        let (_, problems) = track(&ledger, &[work], start + GRACE_SECS);
        assert_eq!(
            problems,
            [
                "project skill work pending for over an hour: /work/brain (other skill source files, unpushed commits)"
            ]
        );

        let (ledger, _) = track(&Ledger::default(), &[project(&[], None)], start);
        let (_, problems) = track(&ledger, &[project(&[], None)], start + GRACE_SECS);
        assert_eq!(
            problems,
            ["project skill work pending for over an hour: /work/brain (no upstream branch)"]
        );

        let mut broken = project(&[], Some(0));
        broken.error = Some("Project skill source is missing".into());
        let (ledger, problems) = track(&Ledger::default(), &[broken], start);
        assert_eq!(
            problems,
            ["project inspection failed: /work/brain (Project skill source is missing)"]
        );
        assert!(ledger.first_seen.is_empty());
    }
}

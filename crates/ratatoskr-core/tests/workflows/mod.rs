//! End-to-end core workflows, grouped by the behavior under test.
use ratatoskr_core::{Settings, add_project, apply, load, plan, project_status, save, sync};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use support::{Fixture, git, git_path, reviewed_resolution};

mod background;
mod configuration;
mod conflicts;
mod deployment;
mod editing;
#[cfg(unix)]
mod private_permissions;
mod projects;
mod rendering;
mod stale_targets;
mod support;
mod validation;

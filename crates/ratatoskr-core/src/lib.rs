mod config;
mod content;
mod engine;
mod git;
mod jobs;
mod notifications;
mod presets;
mod schedule;
mod setup;
mod template;
pub mod updates;

pub use config::{Settings, default_config_path, initialize, load, save};
pub use content::{PreviewFile, PreviewTarget, preview_skill, preview_targets};
pub use content::{
    SkillDocument, SkillEntry, archive_skill, create_skill, disable_skill, list_skills, read_skill,
    write_skill_file,
};
pub use content::{SourceDocument, read_source_file, source_files, write_source_file};
pub use engine::{
    Change, Conflict, ConflictAction, Plan, ResolutionPreview, adopt, apply, apply_reviewed, plan,
    preview_resolution, preview_source_edit, read_conflict_source, resolve_conflict,
    restore_backup,
};
pub use git::{ProjectStatus, add_project, project_status, remove_project, sync};
pub use jobs::{JobRun, JobStatus, detect_helper, job_status};
pub use presets::{
    AgentPreset, AgentTarget, add_agent_target, agent_presets, preview_agent_target,
};
pub use schedule::{Schedule, install_schedule, remove_schedule, schedule_plan};
pub use setup::{clone_content, create_content};

pub use template::{
    SkillAnalysis, SkillBlock, SkillDiagnostic, SkillVariable, TextRange, analyze_skill,
};

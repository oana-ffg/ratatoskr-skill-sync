mod desktop_source;
pub use desktop_source::{DesktopSourceEdit, desktop_source_edit};
mod legacy;
pub use legacy::*;
mod editor;
pub use editor::*;
mod background;
mod config;
mod content;
mod engine;
mod git;
mod jobs;
pub use background::{
    BackgroundNotifications, NotificationKind, record_background_problem, recorded_sync,
};
pub mod tray;
#[cfg(windows)]
pub use schedule::migrate_windows_schedule;
mod notifications;
mod process;
mod protection;
mod secrets;
pub use notifications::AlertSettings;
pub use secrets::SecretReference;
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
    Change, Conflict, ConflictAction, HarnessValidation, Plan, ProfileValidation,
    ResolutionPreview, ValidationContext, ValidationProblem, ValidationReport, adopt, apply,
    apply_reviewed, plan, preview_resolution, preview_source_edit, read_conflict_source,
    resolve_conflict, restore_backup, validate_source,
};
pub use git::{
    ContentStatus, ProjectStatus, add_project, content_status, project_status, push_content,
    remove_project, sync,
};
pub use jobs::{JobRun, JobStatus, detect_helper, job_status};
pub use presets::{
    AgentPreset, AgentTarget, add_agent_target, agent_presets, preview_agent_target,
};
pub use schedule::{Schedule, install_schedule, remove_schedule, schedule_plan};
pub use setup::{clone_content, create_content};

pub use template::{
    TemplateAnalysis, TemplateBlock, TemplateDiagnostic, TemplateVariable, TextRange,
    analyze_global_instructions, analyze_skill,
};

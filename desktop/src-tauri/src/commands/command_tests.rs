use std::collections::BTreeSet;

/// Each of these only touches memory or asks Tauri to do something quick. A new command
/// that touches the disk, a process or the network belongs in `off_main_thread`.
const ALLOWED_ON_MAIN_THREAD: &[&str] = &[
    "runtime_platform",
    "set_app_error_listener",
    "set_desktop_busy",
    "restart_desktop",
    "editor_view",
    "edit_skill_draft",
    "different_preview_lines",
];

/// Each command's name, its signature line, and its whole declaration.
fn commands() -> Vec<(&'static str, &'static str, &'static str)> {
    concat!(
        include_str!("../commands.rs"),
        "\n",
        include_str!("editing.rs"),
        "\n",
        include_str!("updates.rs")
    )
    .split("#[tauri::command]\n")
    .skip(1)
    .map(|command| {
        let signature = command
            .lines()
            .find(|line| line.contains(" fn "))
            .expect("a Tauri command has a signature");
        let name = signature
            .split(" fn ")
            .nth(1)
            .and_then(|rest| rest.split(['(', '<']).next())
            .expect("a Tauri command has a name");
        let (declaration, _) = command.split_once('{').expect("a Tauri command has a body");
        (name, signature, declaration)
    })
    .collect()
}

/// A command moved into another module must still be covered by the policy checks.
#[test]
fn command_policy_checks_cover_every_registered_command() {
    let registered: BTreeSet<_> = include_str!("../main.rs")
        .split("tauri::generate_handler![")
        .nth(1)
        .and_then(|handler| handler.split(']').next())
        .expect("the desktop registers its commands")
        .split(',')
        .map(|command| {
            command
                .trim()
                .strip_prefix("commands::")
                .expect("registered commands belong to the command API")
        })
        .collect();
    let inspected: BTreeSet<_> = commands().into_iter().map(|(name, _, _)| name).collect();
    assert_eq!(registered, inspected);
}

/// A listed command without the parameter gets an unused channel. An unlisted command
/// with it fails every call, and only a native run would notice.
#[test]
fn progress_commands_match_the_frontend_list() {
    let listed: BTreeSet<&str> = include_str!("../../../src/mutation-progress.ts")
        .split("new Set([")
        .nth(1)
        .and_then(|rest| rest.split("])").next())
        .expect("the frontend lists its progress commands")
        .split('"')
        .skip(1)
        .step_by(2)
        .collect();
    let reporting: BTreeSet<&str> = commands()
        .into_iter()
        .filter(|(_, _, declaration)| declaration.contains("on_progress: Channel<String>"))
        .map(|(name, _, _)| name)
        .collect();
    assert_eq!(listed, reporting);
}

#[test]
fn main_thread_commands_stay_trivial() {
    let synchronous: Vec<_> = commands()
        .into_iter()
        .filter(|(_, signature, _)| !signature.contains("async fn"))
        .map(|(name, _, _)| name)
        .collect();
    let unexpected: Vec<_> = synchronous
        .iter()
        .filter(|name| !ALLOWED_ON_MAIN_THREAD.contains(name))
        .collect();
    assert!(
        unexpected.is_empty(),
        "These commands would block the window; move them off the main thread: {unexpected:?}"
    );
    assert!(
        ALLOWED_ON_MAIN_THREAD
            .iter()
            .all(|name| synchronous.contains(name)),
        "Remove stale entries from ALLOWED_ON_MAIN_THREAD: {synchronous:?}"
    );
}

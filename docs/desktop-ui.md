# Desktop UI

The old single-template vanilla TypeScript UI has been replaced by Svelte 5 components and a sidebar router. The Rust core remains authoritative for template syntax, structured commands, YAML/TOML fields, preview rendering and conflict resolution.

## Everyday workflow

1. **Skills:** search the library, select a scope, enable/disable a skill, or create one. Target dots mean inclusion across configured profiles for that agent; they are not a claim that every machine is currently online or synced.
2. **Skill editor:** edit the description, targeting chips and Markdown. The toolbar formats text, wraps selections in conditions, inserts machine values and opens Find (also Ctrl/⌘F anywhere on the screen) with next/previous, match case, regular expressions and replace. Every line inside a condition has a coloured band. Each condition header can edit or remove its markers while keeping the body. A warning marks sections matching no enabled target. Use the agent × machine selectors to compare Rust-rendered output. Installation details show the actual destination; source and destination are never written during preview. Rename and Archive need a saved draft. Archive moves the skill into `.ratatoskr-archive` in its content repository, shows where it went, and returns to the library; installed copies are removed on the next Apply.
3. **Global instructions:** edit the shared `global/AGENTS.md` with the same toolbar, conditions and values as skills. The live preview renders any agent × machine, highlights lines that differ from that machine's other agents, and shows the destination file, such as `~/.claude/CLAUDE.md` or Codex's separate `~/.codex/AGENTS.md`. Saving validates the draft on every machine profile and refuses new problems or blank instructions. The first save creates the file; an agent's existing unmanaged instructions file then appears in Review changes before anything is written.
4. **Review changes:** expand a skill × agent group to inspect dimmed context and word-level side-by-side diffs (source on the left), resolve installed edits with a preview and backup, then Apply the exact reviewed plan. Edit source opens the mapped source, including variants, and uses the core's validated source-edit resolution. Copy-back stays disabled when templates make reversal ambiguous.
5. **Projects:** pick a folder, select its relative content folder and monitor source-only Git status. Each card shows its source skill count, including disabled skills, separately from uncommitted files and unpushed commits. “View skills” opens that project's scope; the library count follows scope and search. An unreadable skill list is reported as unavailable, never as zero. Removing monitoring preserves project files.
6. **Agents & machines:** add a documented preset or custom agent, edit folders and instructions, and configure system, install mode, enabled agents, capabilities and named path values for each profile.
7. **Activity:** inspect completion/result/errors, auto-detected helper, bounded logs and a previewed background schedule.
8. **Settings:** configure signed nightly updates, login startup and ready-made alerts.

Save/Discard appear in the top bar only for editable, unsaved changes in the skill editor, global instructions, machine/agent forms or Settings. Navigation and native close ask before abandoning drafts. Every open form/dialog, queued edit and active mutation blocks automatic installation/restart. Startup update decisions finish before the workspace becomes interactive. Errors are visible; failed saves keep drafts open.

## Snapshot freshness

The header's “As of” label dates the last good workspace snapshot, independently of the last successful sync. The app rereads the workspace on native focus, explicit tray show or macOS Dock reopen, and every 60 seconds while visible. Hidden and minimized windows do not poll. Native focus/resize events and `isVisible`/`isMinimized` are used instead of relying on a hidden WebView's DOM events. The Windows tray emits `workspace-window-visible` after show/unminimize and after hide; macOS Dock reopen uses the same show path. Browser previews use DOM focus/visibility events.

Refreshes share one App-level path and concurrent triggers share the in-flight request. Open editors retain their draft state, current screen and selection. Unsaved edits, queued edits, dialogs and active mutations quietly defer snapshot refreshes. The “As of” label continues to date the displayed snapshot. Save, discard or leaving the protected screen triggers catch-up when visible. If editing starts during a request, its response is discarded. Mutation snapshots supersede older in-flight reads.

A refresh failure keeps the last good data with a persistent error banner, the error text, last good time and a retry button. The header reports unavailable freshness when refresh fails, and the age continues advancing. Activity uses the same workspace jobs snapshot. Windows tray controls read their separate in-process status every two seconds using the same visibility watcher; those reads also coalesce, and preference changes request a shared workspace refresh.

## Source preservation

CodeMirror keeps a normalized display document plus an exact source state. A UTF-16 map accounts for CRLF offsets, including mixed separators and Unicode. Incremental edits preserve untouched separators. Source-state effects participate in undo/redo, so structured commands can be undone without normalizing the file. Decorations never rewrite document text; copying source selections retains template markers. Source mode uses the same document. Structured mode replaces the complete YAML header as a block so long wrapped metadata contributes no hidden line height. Line numbers appear only in Markdown source mode.

Core edits use shared template-parser ranges. YAML scalar ranges come from a concrete syntax tree, preserving comments and unrelated fields. TOML changes use `toml_edit`, reconcile normalized separators with the original source, and change only requested values. No-op edits return the original text. A combined Markdown/metadata save validates both drafts in isolation and checks both original hashes before writes. Each replacement is atomic; the pair is not crash-atomic. A metadata write failure attempts Markdown rollback and reports any failure.

## Decisions where the mockup was silent

- The app is dark-only, including native window chrome and the initial webview background.
- Empty targeting groups mean unrestricted; capabilities require every selected capability. This matches the existing core semantics.
- Preview differences use Rust line alignment, so omitted blocks do not falsely mark identical later lines.
- Rendering is shown as readable monospaced Markdown, with the YAML header hidden by the core. Source mode remains available in the editor.
- The sidebar collapses to icons on narrower windows; the preview moves below the editor on small windows.
- Renaming is explicit and unavailable while a draft is unsaved. Resource imports also require saving/discarding first.
- Legacy Codex copies have a separate decision section. Explicitly retire a copy into a same-volume archive with a receipt, or keep both and leave the older copy unmanaged. Nothing changes until a choice is made.
- Alert secrets use gopass, macOS Keychain, Windows Credential Manager or environment references, resolved for each delivery. Machines with the `gopass` capability default to gopass.

## Verification

`npm test` runs existing async-update tests plus Vitest/Testing Library components and CodeMirror byte round trips. `prepare:fixtures` builds the public Rust fixture oracle. Tests mock Tauri IPC; parsing, commands and previews still run through the real Rust core in disposable directories.

`npm run test:ui` drives the actual editor in Chromium and WebKit. It edits Markdown, crosses three refresh ticks and refocuses, saves while preview is pending, rejects invalid input and retries a repaired draft, refuses a real source hash conflict, and checks excluded preview targets. The bridge sends saves through the Rust core's validation and filesystem writes and reads the saved bytes back; successful saves are not canned responses. CI runs both engines on every PR, retaining traces and screenshots on failure. The fixtures never access the user's configuration or skills.

Save uses the core's validation of the exact submitted draft, independently of live preview timing or stale diagnostics. Failed saves keep the draft and display the core's reason. A new save attempt clears the previous action error. Preview installation details appear only for a completed, successful render that includes the skill.

`npm run test:screenshots` runs Playwright for the dark-only app and writes every main screen to `docs/screenshots/`. It asserts the editor's condition header, value pill, inline unreachable-section warning and live preview, checks realistic long frontmatter and the source-mode gutter, then routes through all screens, expands grouped files, exercises credential controls and rejects page errors. The fixture is public synthetic data, not a snapshot of a personal content repository. Native dialogs, real login registration, live OS notifications, and installed update/relaunch behavior require platform acceptance testing.

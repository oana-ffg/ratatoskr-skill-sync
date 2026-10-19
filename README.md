# Ratatoskr Skill Sync

<img src="desktop/public/ratatoskr.png" width="130" align="right" alt="Ratatoskr, a red squirrel holding an acorn">

One Git source for your AI-agent skills and instructions, adapted to each machine, operating system, and harness.

Ratatoskr is a Rust command-line helper with an optional Tauri 2 desktop app. Windows sync stays resident in the tray when the window is closed; macOS uses a launchd helper. Your content lives in a separate repository, which can be private. The tool is MIT licensed.

## Desktop interface

The Svelte 5 desktop uses a CodeMirror 6 Markdown editor with syntax highlighting, editable conditional headers, variable pills, targeting chips and a Rust-rendered live preview. Description, targeting, agent definitions, machine values and settings have forms. **You never need to edit template tags or TOML by hand.** Markdown source mode is optional. The app is dark-only.

[Desktop walkthrough and design decisions](docs/desktop-ui.md) · [Dark screenshots](docs/screenshots/README.md)

## Features in 0.2.2

- Render machine, OS, harness, and capability conditions; substitute variables; select file variants; add harness-specific frontmatter.
- Preview output diffs and identify hand edits, removals, executable permission changes and unmanaged collisions. Resolve each conflict with a reviewed, backed-up copy-back, reinstall or adoption. Ambiguous templates have a mapped-source editor instead of guessed reverse-mapping.
- Create, clone, or connect a content repository. Create, edit, enable, disable and archive source skills. Validate skill drafts as you type, preview the Rust-rendered result for an enabled machine × harness, and detect stale editor versions.
- Add documented Claude Code, Codex, Gemini CLI, GitHub Copilot CLI and OpenCode presets, or define custom targets through a form. The form writes ordinary source definitions; machine profiles control which tags are enabled. See [verified paths and omissions](docs/agent-presets.md).
- Render global and registered project skills. Report uncommitted and unpushed project skill-source work.
- Use tray-resident sync on Windows or a per-user launchd job on macOS. Each run fast-forwards a clean content checkout and applies it. System, Pushover, ntfy or command-based failure/recovery alerts are deduplicated. View the last run, current ledger errors and bounded logs in the app; helper paths are detected automatically.
- Check and install cryptographically signed helper and desktop updates, with nightly automatic updates enabled by default. A release workflow builds signed draft releases for Apple Silicon, Intel Mac, and Windows x64.

**Pre-1.0:** version 0.2.2 fixes structured-editor layout with long frontmatter, groups review work by skill and agent, and adds secret-store references for alerts. Apple/Microsoft publisher signing is not configured. Start in staging and review the [verification record and remaining limits](docs/verification.md).

## Build and run

Install stable Rust (1.90 or newer), Git, Node.js 22.18 or newer, and [Tauri's platform prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
cargo build --locked
cargo run -- --help
cd desktop
npm ci
npm run tauri dev
```

Windows uses the desktop app for tray-resident sync. `cargo build --locked` also builds the standalone console CLI.

To package a native app, run `npm run tauri build` inside `desktop`. The build prepares and bundles the independent helper. The browser-only `npm run dev` preview has no filesystem access. The screenshot tests install mocked IPC using disposable public fixtures, never your real configuration.

The executable is `ratatoskr`. `init --create` creates a Git content repository with a profile for the current host, staging enabled, and Claude/Codex presets. Choose an absolute source directory and a lowercase machine identifier. `init` without `--create` connects an existing checkout; `--remote` clones a Git URL into a new directory. The selected machine must exist under `machines/`. Initialization never applies outputs.

After connecting:

```sh
ratatoskr plan
ratatoskr apply
ratatoskr status
```

Use `--config` to select a local configuration or set the absolute `RATATOSKR_CONFIG` environment variable. Both the desktop app and helper respect it. `--json` emits structured plans and status. `status` returns 1 for pending changes, conflicts, project inspection problems, or uncommitted/unpushed skill work; operational errors return 2.

## Validate a content checkout

Before opening a content PR, render every profile on any host:

```sh
ratatoskr validate ./agentfiles
ratatoskr validate ./agentfiles --json
```

`validate` needs no local configuration. It reads every `machines/*.toml` profile and uses the sync renderer for each enabled harness, selected skill and optional `global/AGENTS.md`. It reports independent template, variable, metadata, target, variant and portable-path errors with machine/harness/skill/file context, continuing after failures. Summaries include each harness's rendered/skipped skill counts, file counts and global instructions.

Rendering stays in memory: no state directory, staging copies, output writes, Git commands or network access. Foreign live profiles are supported without resolving their homes on the local filesystem. This checks generic rendering; it does not run language-specific syntax checks, inspect installed-file ownership or validate host permissions/symlinks at foreign destinations. Use `plan` on the intended machine for deployment checks.

Exit codes are 0 when every profile renders cleanly, 1 for reported validation problems, and 2 when the source cannot be inspected (for example, a missing checkout or no profiles). Identical problems (same path and message) appear once, with their first context; per-profile counts still include every failed render attempt. `--json` emits the full profile summaries and problem list for completed validation; inspection failures go to stderr.

## Development checks

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cd desktop
npm ci
npm run format:check
npm test
npm run check
npm run build
npx playwright install chromium
npm run test:screenshots
```

`cargo test --workspace` and non-bundling Cargo checks do not need a prebuilt helper. When it is absent, the build excludes the sidecar from that non-bundling build and reports how to prepare it. macOS background jobs require the real CLI (`npm run prepare:helper -- --debug` from `desktop/`). Windows tray sync calls the core directly. Native packaging still requires and bundles the real CLI; the Tauri CLI prepares it automatically. Rust's platform prerequisites still apply to workspace tests.

## Content and projects

Content contains `harnesses.toml`, `machines/<machine>.toml`, `skills/<name>/SKILL.md` plus `skill.toml`, and optional `global/AGENTS.md`. See the [format reference](docs/content-format.md).

Machine profiles default to **staging**, which writes beneath Ratatoskr's state directory. A live profile must match the current OS and home. Local configuration selects the source checkout, machine, state directory, and project registry.

From a project's Git root, create an `agent-skills/skills/` source folder, then run:

```sh
ratatoskr project add "$PWD"
```

`--source` chooses another portable relative source directory. Registration creates `.ratatoskr.toml`; commit it alongside skill sources. Project skills render into each harness's configured `project_root` (default `.<harness>`) and `skills_dir`. Global instructions are not rendered into projects. Staging keeps project output beneath Ratatoskr's state directory. Unregistering preserves the marker, source, and generated output, and releases ownership of that project's output.

## Background sync and recovery

Use a dedicated clean deployment checkout with a Git upstream for scheduled syncing. Keep authoring in a separate checkout. Local `deploy_branch` defaults to `main`. Before updating the helper, pulling, or applying, `ratatoskr sync` requires that exact current branch and its `<remote>/<deploy_branch>` upstream. Experimental branches, wrong or missing upstreams, and detached HEADs refuse deployment and alert. It then pulls with `--ff-only`, applies, and exits; dirty or divergent content history is also refused.

On Windows, the desktop app syncs directly through the Rust core every 60 seconds while running. Closing the window hides it to the tray. Left-click the tray icon to open it; the menu offers Open, Sync now, Pause/Resume sync, and Quit. Quit stops syncing. Pause stops automatic runs; Sync now still requests one run. **Settings → Background sync** changes the interval (60–86400 seconds) or pauses it. **Open Ratatoskr at login** uses the existing per-user Run key and starts hidden. A second app launch reopens the existing instance.

The tray shows active, paused, or error status, the last successful sync time, and skill count. Windows toasts deduplicate stable problem identities separately from full diagnostics. A persisted, shared limit allows at most one problem toast per hour across all sources; recovery remains allowed. Unchanged failures never repeat, and there are no modal dialogs. Native sync and nightly-update timers continue while the window is hidden. Updates preserve background/foreground launch state and defer installation while the UI has unsaved work.

At startup, Windows migrates an existing task only when its ownership record, configuration identity, and byte-exact definition are verified. Before removing an owned task, migration enables hidden Open Ratatoskr at login to retain the old logon trigger and records the migration in the ledger. A login-start failure leaves the task intact. Enabled and disabled tasks use the same safe removal. An unowned or edited task is preserved; sync is blocked and the problem appears in Settings, Activity, and `job-status`. **Retry task migration** repeats detection after manual resolution. `schedule plan` and `schedule install` now fail with a tray-mode instruction on Windows; `schedule remove` remains available for safely removing an owned legacy task.

On macOS, `ratatoskr schedule plan` previews a launchd job using the detected helper. `--helper` selects an absolute helper path; `schedule install` registers it and `schedule remove` removes only the recorded, unchanged job. `--seconds` accepts 60–86400 (default 60). The desktop exposes these operations in Activity. The launchd plist bytes and scheduling behavior are unchanged. Pending project skill commits or pushes return a sync error.

**Settings → Alerts** provides system notifications, Pushover, ntfy and a custom command. Pushover credentials and optional ntfy bearer tokens are read from named environment variables in the helper’s environment; local settings store those names, not credential values. System delivery uses macOS notifications, Windows toast notifications or Linux `notify-send`. Delivery failures remain visible and retryable.

`alert_command` in local configuration is an argv array. Ratatoskr appends a plain-English message and executes it without a shell. Configured alert transports retain their existing policy: new failures alert immediately, unchanged failures repeat after 24 hours, and recovery alerts once. Windows tray toasts use stable-identity dedupe and the shared one-hour problem-toast limit instead. Failed delivery remains an error. Without an alert command, failures still return nonzero and persist in the error ledger and run history. `ratatoskr job-status` returns the last start/completion/result, sync and updater ledger errors, the last 64 outcomes and the last 64 KiB of each scheduler log. A start without a completion record is shown explicitly.

Tray runs write reports to `sync.stdout.log`, errors to `sync.stderr.log`, and every started run to the existing job ledger/history. Logs are bounded; lock-busy attempts defer without overwriting another run. A broken configuration uses a safe bootstrap ledger beside the config, under `<config filename>.diagnostics/startup/`. Diagnostic failures appear in the tray status and stderr; no modal is shown. macOS retains launchd logs. The obsolete Windows background helper is no longer built or packaged. See [upgrade compatibility](docs/releases.md#windows-tray-migration).

For existing unmanaged imports, the legacy bulk `ratatoskr adopt` backs up existing matching target directories and global files with corresponding source content before recording ownership. It does not write generated output. Review `plan` before applying: adopted files absent from source are scheduled for removal. `ratatoskr restore` accepts the backup directory printed by adoption, verifies every stored hash, and refuses current hand edits or unmanaged overwrite. Archiving a source skill moves it into `.ratatoskr-archive` on the same volume.

## Codex legacy cut-over

Codex's preset installs skills in `.agents/skills`. Codex also discovers `~/.codex/skills`; same-named copies can both appear. When Codex is enabled, **Review changes** lists legacy duplicates even in staging. Live Apply (including CLI/background sync) refuses to proceed until you explicitly retire the old copy or choose **Keep both copies**. Nothing changes before that choice.

The explicit **Back up and retire the old copy** action previews every file and fingerprint, including runtime artifacts. **Back up and retire old copy** moves the entire old directory on the same volume into `~/.codex/ratatoskr-backups/`, outside scanned skill folders, and saves a JSON receipt, including any prior Ratatoskr ownership records for that skill. Archived paths are released from the old ownership manifest so the new target can be applied. No files are silently deleted. Changed files or a redirected backup invalidate the preview. For manual recovery, move the archived directory back to the original path recorded in its receipt after checking for an existing destination. This archive is separate from `ratatoskr restore` workspace backups.

**Keep both copies** preserves every legacy file and releases any old Ratatoskr ownership so that copy stays unmanaged. The decision is local to this workspace and records the reviewed file hashes, permissions and configured destination. A changed old copy or destination needs a new decision.

Alert credentials use references to **gopass**, **macOS Keychain**, **Windows Credential Manager**, or **environment variables**. The Settings form defaults to gopass when the selected machine declares that capability. Only names are stored locally; credentials are resolved for each delivery, and resolution failures stay visible in Activity and scheduler output. Existing environment-name settings migrate when saved. See [alert reference fields](docs/desktop-api.md#structured-alert-settings).

## Resolve conflicts

The desktop shows each conflict's cause, installed-versus-rendered diff and mapped source. Preview an action, inspect its file effects, then explicitly choose **Back up and execute this action**. A changed source or destination invalidates the preview.

- **Copy edit back to source:** available only for one mapped file without conditional comments, variables or injected frontmatter. The core validates the installed bytes as source before writing; a chosen variant maps to its actual variant file. Other targets remain pending until Apply is reviewed.
- **Discard edit and reinstall:** backs up current bytes and ownership, then replaces only the selected conflict with rendered source. For an unmanaged directory, the preview includes extra files that will be backed up and removed.
- **Take ownership as-is:** backs up and records current bytes or absence. Differences from source remain pending; a later Apply replaces adopted content.
- **Edit mapped source:** for templates, edit the actual source directly, validate and preview the source diff, then back up and save. This leaves installed files and their ownership unchanged; review the conflict again afterwards.

CLI equivalents use the same core. `conflict preview` always emits JSON; save it for the explicit second step:

```sh
ratatoskr conflict preview "$installed_file" --action reinstall > resolution.json
ratatoskr conflict resolve --preview resolution.json
```

Actions are `copy-back`, `reinstall` and `adopt`. `conflict source "$installed_file"` reads its mapped source; `conflict edit-source "$installed_file" --input "$edited_source"` validates and emits a manual-edit preview for the same `conflict resolve` step. `restore` accepts the backup directory printed by resolution. Hashes, source mappings and destination boundaries are verified before restoration; resolve other outstanding conflicts before restoring. Restoration reinstates original ownership, so a restored hand edit is visible as a conflict again.

`ratatoskr skill preview "$skill_name" --machine "$machine" --harness "$harness"` renders saved content. `--markdown` and `--metadata` accept draft files; `--project` selects a registered project source. The desktop debounces both editors and ignores stale asynchronous results. Binary output is labelled; no template logic runs in TypeScript.

## Updates and operating limits

Official GitHub Releases and the trusted public signing key are built into both applications. No per-machine key setup is needed. Automatic updates default to **00:00 local time**; change the time or switch them off in **Settings → Updates**. The equivalent configuration is:

```toml
[updates]
automatic = true
time = "00:00"
```

The desktop **Open at login** toggle uses Tauri autostart and defaults off for a new install. Windows migration from an owned task enables it to preserve logon syncing; otherwise the existing choice is retained. Windows Background sync warns when it is off. On macOS it remains separate from the helper schedule.

The running desktop checks once during the one-hour nightly window. An available update waits for unsaved editors and apply/sync/schedule work to finish, then installs and relaunches the connected workspace. No available update means no restart. If work stays busy until the window closes, it tries the next night or at app launch. Launch catches up on a missed nightly window, installing and relaunching before the workspace becomes interactive. Checks are persisted across launches. The standalone helper records its last attempt and checks at most once per local day, at or after the chosen time, before that run's sync. Its next run uses the new binary. Update failures have separate deduplicated alerts and never prevent content syncing. Bundled helpers update with the desktop.

Forks may explicitly set `updates.endpoint` and/or `updates.public_key`. The UI prominently labels active overrides. See [release operations](docs/releases.md) for the signing trust root and GitHub secrets. Cryptographic update signatures are separate from Apple notarization and Windows publisher signing.

The ownership manifest belongs to one source and machine. Symlinks, traversal, case collisions, overlapping roots, conflicting variants, and malformed source files fail validation. There is no force-overwrite option. An OS lock serializes Ratatoskr applies; the desktop additionally refuses a plan that changed after review.

Each file and manifest replacement is atomic; the entire output tree is not a transaction. An I/O error midway can leave partial changes, which the old manifest exposes as conflicts. Other editors can still race the helper. Backups are not automatic snapshots of every sync.

Project Git status covers its registered source and marker, using local upstream references without fetching. A missing upstream stays visible. `sync` contacts the content upstream, but never resets, stashes, rebases, or force-pushes.

[Desktop command API](docs/desktop-api.md) · [Architecture](docs/design.md) · [Branding](docs/branding.md) · [Contribution rules](AGENTS.md)

# Using Ratatoskr

Reference for the CLI, background sync, conflict resolution and updates. For an overview, see the [README](../README.md).

## Initialize and run

The executable is `ratatoskr`. `init --create` creates a Git content repository with a profile for the current host, staging enabled, and Claude/Codex presets. Choose an absolute source directory and a lowercase machine identifier. `init` without `--create` connects an existing checkout; `--remote` clones a Git URL into a new directory. The selected machine must exist under `machines/`. Initialization never applies outputs.

After connecting:

```sh
ratatoskr plan
ratatoskr apply
ratatoskr status
```

Use `--config` to select a local configuration or set the absolute `RATATOSKR_CONFIG` environment variable. Both the desktop app and helper respect it. On initialization, the platform-default config keeps the platform-default state directory. A custom config defaults to an absolute `<config filename>.state/` directory beside that config, including when selected through `RATATOSKR_CONFIG`. `init --state-dir <DIR>` explicitly selects a different absolute state directory for connect, `--create` or `--remote`. Existing configs keep their saved `state_dir`; loading them does not recalculate it. `--json` emits structured plans and status. `status` returns 1 for pending changes, conflicts, project inspection problems, or uncommitted/unpushed skill work; operational errors return 2.

## Disposable preview configuration

For `plan` or `skill preview` from a content PR checkout, create a separate temporary directory for the config and state. On macOS, use `/private/tmp` because `/tmp` is a symlink and Ratatoskr rejects symlinked paths:

```sh
preview_dir="$(mktemp -d /private/tmp/ratatoskr-preview.XXXXXX)"
ratatoskr --config "$preview_dir/config.toml" init --source "$PWD" --machine brunnr
ratatoskr --config "$preview_dir/config.toml" plan
ratatoskr --config "$preview_dir/config.toml" skill preview grill-me --machine brunnr --harness codex
```

Run this from the content checkout with an existing `brunnr` profile and `grill-me` skill, or substitute identifiers present in your source. State is isolated at `$preview_dir/config.toml.state`; add `--state-dir "$preview_dir/preview-state"` to `init` to choose it explicitly. Each config filename gets its own derived state directory, even when multiple configs are in the same directory. These commands do not apply outputs; `apply` still follows the machine profile's staging/live mode, so an isolated state directory alone does not sandbox live destinations.

## Validate a content checkout

Before opening a content PR, render every profile on any host:

```sh
ratatoskr validate ./agentfiles
ratatoskr validate ./agentfiles --json
```

`validate` needs no local configuration. It reads every `machines/*.toml` profile and uses the sync renderer for each enabled harness, selected skill and optional `global/AGENTS.md`. It reports independent template, variable, metadata, target, variant and portable-path errors with machine/harness/skill/file context, continuing after failures. Summaries include each harness's rendered/skipped skill counts, file counts and global instructions.

Rendering stays in memory: no state directory, staging copies, output writes, Git commands or network access. Foreign live profiles are supported without resolving their homes on the local filesystem. This checks generic rendering; it does not run language-specific syntax checks, inspect installed-file ownership or validate host permissions/symlinks at foreign destinations. Use `plan` on the intended machine for deployment checks.

Exit codes are 0 when every profile renders cleanly, 1 for reported validation problems, and 2 when the source cannot be inspected (for example, a missing checkout or no profiles). Identical problems (same path and message) appear once with every affected machine/harness/skill context. Only failing contexts appear, so a missing `paths.*` variable identifies the machine profiles that need attention. Human diagnostics and JSON problem `path` fields use paths relative to the source checkout with forward slashes on every OS, such as `skills/review-code/SKILL.md`. Human contexts look like `[brunnr/claude/grill-me, brunnr/codex/grill-me]`; global instructions and harness configuration contexts omit the skill, and profile-level contexts omit the harness too.

Each profile counts **problem occurrences**: one per distinct problem and affected context. The final summary reports both distinct problems and total occurrences; total occurrences equal the sum of the profile counts. For example, one shared error affecting two harnesses on three profiles produces two occurrences per profile and `3 profiles; 1 distinct problem; 6 problem occurrences` overall. `--json` uses the same grouping: each `problems` entry has `path`, `message` and `contexts: [{machine, harness, skill}]` (unavailable harness/skill values are `null`). Profile summaries and the report both expose `problem_occurrences`; `problems.length` is the distinct count. Inspection failures go to stderr.

## Content and projects

Content contains `harnesses.toml`, `machines/<machine>.toml`, `skills/<name>/SKILL.md` plus `skill.toml`, and optional `global/AGENTS.md`. See the [format reference](content-format.md).

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

Tray runs write reports to `sync.stdout.log`, errors to `sync.stderr.log`, and every started run to the existing job ledger/history. Logs are bounded; lock-busy attempts defer without overwriting another run. A broken configuration uses a safe bootstrap ledger beside the config, under `<config filename>.diagnostics/startup/`. Diagnostic failures appear in the tray status and stderr; no modal is shown. macOS retains launchd logs. The obsolete Windows background helper is no longer built or packaged. See [upgrade compatibility](releases.md#windows-tray-migration).

For existing unmanaged imports, the legacy bulk `ratatoskr adopt` backs up existing matching target directories and global files with corresponding source content before recording ownership. It does not write generated output. Review `plan` before applying: adopted files absent from source are scheduled for removal. `ratatoskr restore` accepts the backup directory printed by adoption, verifies every stored hash, and refuses current hand edits or unmanaged overwrite. Archiving a source skill moves it into `.ratatoskr-archive` on the same volume.

## Codex legacy cut-over

Codex's preset installs skills in `.agents/skills`. Codex also discovers `~/.codex/skills`; same-named copies can both appear. When Codex is enabled, **Review changes** lists legacy duplicates even in staging. Live Apply (including CLI/background sync) refuses to proceed until you explicitly retire the old copy or choose **Keep both copies**. Nothing changes before that choice.

The explicit **Back up and retire the old copy** action previews every file and fingerprint, including runtime artifacts. **Back up and retire old copy** moves the entire old directory on the same volume into `~/.codex/ratatoskr-backups/`, outside scanned skill folders, and saves a JSON receipt, including any prior Ratatoskr ownership records for that skill. Archived paths are released from the old ownership manifest so the new target can be applied. No files are silently deleted. Changed files or a redirected backup invalidate the preview. For manual recovery, move the archived directory back to the original path recorded in its receipt after checking for an existing destination. This archive is separate from `ratatoskr restore` workspace backups.

**Keep both copies** preserves every legacy file and releases any old Ratatoskr ownership so that copy stays unmanaged. The decision is local to this workspace and records the reviewed file hashes, permissions and configured destination. A changed old copy or destination needs a new decision.

Alert credentials use references to **gopass**, **macOS Keychain**, **Windows Credential Manager**, or **environment variables**. The Settings form defaults to gopass when the selected machine declares that capability. Only names are stored locally; credentials are resolved for each delivery, and resolution failures stay visible in Activity and scheduler output. Existing environment-name settings migrate when saved. See [alert reference fields](desktop-api.md#structured-alert-settings).

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

The running desktop uses one shared Rust scheduler on macOS and Windows, independent of hidden or suspended WebViews. It reevaluates the local clock every minute and checks once per nightly window. If sleep or a delayed tick misses the window, the next tick catches up and opens one persisted hour for installation. **This catch-up can install the update and relaunch the app during the daytime**, once unsaved edits and running operations are clear. This extends automatic installation beyond the nightly window and app launch. An available update waits for unsaved editors and apply/sync/schedule work, including edits started during download. If that hour expires while busy, installation waits for the next night or app launch; retries never extend the hour. No available update means no restart. Startup catches up before the workspace becomes interactive. Attempts, successful checks, and pending installation state are persisted across launches.

Manual **Check** records a successful check but does not open a catch-up hour or queue the discovered update for automatic installation at the next launch. Use **Install** to install it manually; the independent automatic schedule still applies.

Agents and scripts update the desktop app without touching its UI. Use the bundled CLI (`/Applications/Ratatoskr Skill Sync.app/Contents/MacOS/ratatoskr` on macOS, `ratatoskr.exe` beside the Windows app):

```sh
ratatoskr update --desktop           # queue a signed update; returns immediately
ratatoskr update --desktop --status  # last request and its result; exit 1 if it failed
```

The running app picks the request up within seconds. It uses the same signed updater, unsaved-edit wait and apply/sync/schedule locks as nightly installation, but ignores the nightly window and the automatic-updates switch. It then restarts with its window state preserved. If the app is not running, the command starts it hidden; no window opens or takes focus. On Windows the installer shows its usual brief progress bar. The result reads `already up to date`, `installing <version>` (until the restarted app confirms it), `installed <version>` or `failed`. A request waits, with no result yet, while edits or operations are in progress. Failures also appear in `job-status` and alerts.

Two completed nightly windows without a successful check produce a desktop-update error in Activity and `job-status`, delivered through the configured alert transport (including Pushover) with the existing 24-hour dedup/recovery policy. A specific check or installation error takes precedence over the generic overdue message. Attempts and idle ticks do not clear the error; a successful check does. Existing ledgers begin a monitoring grace period because old attempt timestamps do not prove success. Monitoring restarts at app launch so time with the app closed does not count as a resident failure.

The standalone helper records its last attempt and checks at most once per local day, at or after the chosen time, before that run's sync. Its next run uses the new binary. Update failures have separate deduplicated alerts and never prevent content syncing. Bundled helpers update with the desktop.

Forks may explicitly set `updates.endpoint` and/or `updates.public_key`. The UI prominently labels active overrides. See [release operations](releases.md) for the signing trust root and GitHub secrets. Cryptographic update signatures are separate from Apple notarization and Windows publisher signing.

The ownership manifest belongs to one source and machine. Symlinks, traversal, case collisions, overlapping roots, conflicting variants, and malformed source files fail validation. There is no force-overwrite option. An OS lock serializes Ratatoskr applies; the desktop additionally refuses a plan that changed after review.

Each file and manifest replacement is atomic; the entire output tree is not a transaction. An I/O error midway can leave partial changes, which the old manifest exposes as conflicts. Other editors can still race the helper. Backups are not automatic snapshots of every sync.

Project Git status covers its registered source and marker, using local upstream references without fetching. A missing upstream stays visible. `sync` contacts the content upstream, but never resets, stashes, rebases, or force-pushes.

[Desktop command API](desktop-api.md) · [Architecture](design.md) · [Branding](branding.md) · [Contribution rules](../AGENTS.md)

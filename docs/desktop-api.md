# Desktop command API (0.2.2)

PR #2 provides the Rust implementation, CLI equivalents and Tauri endpoints for the UI redesign. The desktop uses Svelte 5 and CodeMirror 6; structured edits use the same Rust core as the CLI. All command failures propagate as rejected invocations with contextual error text.

## Central source publication

The source-writing commands `save_global_instructions`, `save_skill_draft`, `set_skill_enabled`, `archive_source_skill`, `rename_source_skill`, `create_desktop_skill`, `add_skill_resource`, `save_structured_source`, `add_agent_target`, `resolve_conflict` (source actions), and `restore_workspace` (source restoration) run on a blocking worker and use the shared desktop source transaction. Central edits commit only actually changed paths and push the deploy upstream under `sync.lock` → `apply.lock`; monitored-project edits and CLI writers remain write-only. Setup (`connect_workspace`) initializes/connects/clones before an existing content workspace is available and does not publish edits.

`read_workspace().plan.content_status` and `read_content_status()` expose `{changed_files, outgoing_commits, upstream, error}`. Unknown counts/upstreams remain nullable and inspection errors stay visible. The small status endpoint lets the UI update publication state after a rejected save without replacing a dirty editor. `push_content()` validates the deploy branch/upstream and pushes existing commits under the same locks, returning a snapshot; it never stages or commits files. Commit failures retain source bytes; push failures retain the local commit and reject with `saved and committed locally but NOT pushed: <reason>`.

See [desktop save/freshness behavior](usage.md#desktop-saves-and-content-git-state). Source hashes and serialized conflict previews are revalidated after a successful clean-checkout fast-forward. Fetch/fast-forward failures stop before writing.

## Workspace reads

`read_workspace()` rebuilds the snapshot from current on-disk settings, machine mode, plan, job history and legacy copies; it has no resident snapshot cache. The frontend serializes automatic reads, records their start time for the header, defers them while drafts or mutations are active, and visibly retains the last good time on failure. Activity consumes this shared snapshot rather than a separate jobs read.

The snapshot's `skills_error` is null when its skill list was read successfully. A non-null message means `skills` is unavailable, not an empty list; the UI shows the error and suppresses numeric skill counts.

`workspace-window-visible` is a native window event with a boolean payload. The shared native open path emits `true` after show/unminimize/focus; macOS close-to-hide and Windows close-to-tray emit `false` after hiding. Native focus and resize events additionally cover ordinary focus and minimize/restore. The frontend confirms native visibility before refreshing.

## Conflict review and recovery

- `preview_conflict({path, action})` returns a `ResolutionPreview`; actions are `copy-back`, `reinstall`, `adopt`. Copy-back is refused for variables, conditional comments, injected frontmatter, missing installed files and missing source mappings.
- `read_conflict_source({path})` returns the actual mapped source document, including selected variant files and global instructions. Binary files reject text editing.
- `preview_source_edit({path, text})` validates a manually edited source draft and returns an `edit-source` resolution preview.
- `resolve_conflict({expected})` accepts the **complete serialized JSON preview**, recomputes it under the operation lock, backs up actual installed bytes, missing-file state, original ownership and any affected source, and returns the backup directory. It rechecks again after backup. Do not reconstruct or edit preview fields in the client.
- `restore_workspace({backup})` verifies the existing backup format plus the optional source/ownership extensions, restoring the prior state. A source changed after the backed-up action is refused. Current conflicts must first be resolved. Restored hand edits reopen conflicts.

The snapshot plan retains human-readable `conflicts` for CLI compatibility and adds `conflict_details` containing `path`, `kind`, `diff`, mapped `source` and `copy_back_blocked`. Kinds are `hand-edited`, `removed`, `permission-change`, `unmanaged-collision`, and `unmanaged-directory`. Directory actions enumerate individual effects. Binary/large text diffs are null; effect hashes and executable state remain available. Adoption records ownership as-is; pending source differences are still replaced by a later reviewed Apply.

## Former target decisions

`read_workspace().plan.stale_targets` contains former locations with `path`, nullable `harness`, `files`, and previously owned skill `roots`. File records include absolute `path`, `owned`, `modified`, recorded/current fingerprints (nullable for untracked/missing files), and current protection metadata. `modified` flags byte or executable-state differences from the ownership manifest. Untracked files inside archived skill directories are also enumerated. Directory permissions and the exact archive roots are part of the verified payload. These decisions also appear in `plan.conflicts`, blocking Apply and existing background sync/alerts without degrading the snapshot to “Preview failed”.

- `list_stale_targets()` returns the same location list independently.
- `preview_stale_target({path, action})`, where action is `forget` or `remove`, returns `StalePreview` with the location, action, nullable archive destination, and a revision binding the ownership and configuration.
- `resolve_stale_target({expected})` accepts the complete serialized preview, verifies it under `apply.lock`, and returns the archive path for Remove or null for Forget. It runs on a blocking worker under the desktop mutation guard. Do not reconstruct or edit preview fields. A successful resolution needs a new workspace read.

Forget releases only reviewed ownership. Remove uses same-volume renames and a JSON recovery receipt, including edited and untracked bytes inside owned skill directories. Unrelated skills are preserved. It releases ownership only after moves succeed, rolling moves back if a later operation fails and reporting exact paths for incomplete rollback. Recovery is manual using the receipt's `moves` mapping; `restore_workspace` retains its current-target boundary and does not accept these archives. See [CLI and recovery instructions](usage.md#former-target-locations).

## Rendering and editor analysis

- The `read_workspace` snapshot's `preview_targets` lists enabled `{machine, harness}` pairs from all source profiles.
- `preview_skill({name, project, markdown, metadata, machine, harness})` renders an isolated draft with the deployment renderer. Returns resource-relative paths and text, with null text for binary output. Empty results mean the skill is disabled or excluded. Template context preserves the selected profile's values; destinations stay isolated even for a different OS/live profile.
- `analyze_skill({name, markdown, machine, harness})` returns `blocks`, `variables`, and `diagnostics` using the **same parser, variable resolver and frontmatter validator** as rendering. Every range is half-open `{from, to}` measured in **UTF-16 code units**, suitable for CodeMirror/JavaScript string offsets. Blocks include whole/opening/body/closing ranges, selector arrays, selected-context activity and reachability across enabled machine × harness contexts. Variables include their name, selected-context activity and resolved scalar value or null. Diagnostics include severity (`error` or `warning`), message and range.

Analysis diagnoses parser errors, unknown active variables, malformed substitutions, invalid YAML/identity/description and blocks matching no enabled target. It analyzes the supplied Markdown independently of `skill.toml` targeting; use `preview_skill` to validate metadata, variants and final selection. Frontmatter diagnostics use the frontmatter range; errors without a narrower source location use the document range. Neither analysis nor preview writes the real source or installed output.

## Global instructions

- `read_global_instructions()` returns `{file, text, hash}` for `global/AGENTS.md`. A missing file reads as empty text; its hash creates the file on save.
- `analyze_global_instructions({markdown, machine, harness})` returns the same `blocks`, `variables` and `diagnostics` shape as `analyze_skill`, without frontmatter checks.
- `preview_global_instructions({markdown, machine, harness})` renders the draft with the deployment renderer for one enabled target. An empty string means that target receives an empty file.
- `preview_global_instructions_path({machine, harness})` gives the target's instructions file, including a live `global_root` such as Codex's.
- `save_global_instructions({markdown, expectedHash})` refuses stale hashes, blank text and conditional-block errors, then validates the draft for every machine × harness under the operation lock. As with `save_skill_draft`, the current machine must render without errors, including in unrelated skills; on other profiles only problems introduced in `global/AGENTS.md` block the save. Returns a snapshot.

`write_source_file` refuses `global/AGENTS.md` because it validates only the current machine.

## Agent definitions

- The snapshot's `presets` lists documented defaults with official source URLs.
- `preview_agent({target})` returns a source document with candidate `harnesses.toml` text and the original content hash. Existing text/comments are preserved, with the new definition appended. Duplicate tags reject.
- `add_agent_target({target, expectedHash})` validates and saves the definition using the expected source hash, returning a snapshot. Enable its tag in intended machine profiles separately.

`target` contains `tag`, `global_folder`, `project_folder`, `instructions_file`, and nullable `instructions_folder`. The last field supports Codex's separate instruction location. Preset scope is global skills/instructions plus project skills; this release does not generate project instructions. See [verified preset paths](agent-presets.md).

## Background work and login

- `read_job_status()` returns `last_run` (start/completion Unix seconds and error), `current_error` from the sync dedup ledger, separate updater errors, and bounded logs/history. A missing completion is explicit. All errors are persisted even without a notifier, and failed notification deliveries remain retryable.
- The snapshot's `helper` is the executable bundled beside the desktop or an executable standalone helper found on absolute PATH entries; a missing helper is reported in `helper_error`.
- `preview_schedule({seconds})` and `enable_schedule({seconds})` detect the helper themselves; clients cannot substitute another helper between preview and installation by submitting a path. `disable_schedule()` preserves existing schedule ownership checks.
- `login_enabled()` reads the Tauri autostart state. `set_login_enabled({enabled})` changes it explicitly and returns the verified state. Plugin registration never enables autostart by itself; default is off.

Existing `read_workspace`, configuration, project, sync/apply and update commands remain available. Nightly updates must continue to guard all unsaved editor/form state in the replacement UI.


## Structured desktop editing

- `editor_view({markdown, metadata})` returns description, UTF-16 frontmatter range and parsed metadata values. It does not rewrite source.
- It also returns `description_sections` (ordered shared/conditional text with nullable selectors) and `description_error` for malformed description conditions, keeping source mode available for repair. `edit_skill_draft` accepts `description-section` with an `edit` of `text` (`index`, `value`), `append` (`value`, `selectors`), `only` (`index`, `selectors`), or `remove-only` (`index`). Indices address the current decoded description sections; Rust preserves all other source fields and body bytes.
- `edit_skill_draft({markdown, metadata, edit})` returns draft Markdown/metadata only. `edit.kind` is `description` (`value`), `metadata` (`path`, nullable typed `value`), `only` (`from`, `to`, `selectors`, `existing`), `remove-only` (`from`), or `variable` (`from`, `to`, `name`). Conditions use the shared parser. All offsets are UTF-16 source offsets, before display newline normalization.
- `save_skill_draft({name, project, markdown, metadata, markdownHash, metadataHash})` validates both drafts together, checks both hashes, and saves only changed files under the operation lock. A second-file failure rolls back the first file when possible; process termination is not a two-file transaction.
- `create_desktop_skill({name, project, description})` constructs valid frontmatter in Rust. `rename_source_skill({name, project, newName, expectedHash})` explicitly renames the directory and identity. `archive_source_skill({name, project})` moves the whole skill directory into `.ratatoskr-archive` under the operation lock, without deleting anything, and returns `{path, workspace}`.
- `read_structured_source({file})` returns `{file, hash, values}`. `save_structured_source({file, hash, values})` changes only differing fields and validates the candidate content before saving. Existing comments, unrelated values and separators are preserved.
- `read_local_settings()` returns `{settings, hash}`. `save_local_settings({candidate, expectedHash})` rejects stale configuration and preserves unrelated source formatting. Workspace/project locations require their dedicated actions.
- `list_values({machine, harness})`, `list_skill_resources({name, project})` and `skill_target_statuses()` provide the pickers, file panel and library inclusion indicators.
- `add_skill_resource({name, project, file, from})` imports a new resource/variant after isolated validation; existing files are refused.
- `preview_skill` files add `body` for the core-stripped Markdown body of SKILL.md. `preview_install_path({name, project, machine, harness})` gives the selected profile's real installation destination. `different_preview_lines({text, versions})` returns aligned line indices for preview highlighting.
- `choose_folder()` and `choose_resource()` use native dialogs, returning a selected path or null on cancellation.

## Legacy Codex cut-over

Snapshots include `legacy` duplicates when an enabled local Codex profile installs outside the legacy skill root. `preview_legacy_resolution({name})` returns every file's SHA-256/executable state and the proposed same-volume archive destination. `resolve_legacy({expected})` accepts the complete serialized preview, rechecks it under the operation lock, saves a receipt and renames the full directory. The receipt includes any selected legacy ownership; only archived paths are detached, and a failed ownership update rolls the rename back. It refuses changed content, symlinks, existing backups or redirected archives. `keep_legacy({expected})` accepts the serialized duplicate, rechecks its complete fingerprint under the operation lock, releases any old ownership without modifying the copy, and records the choice in local state. Changed legacy bytes, permissions or a changed destination invalidate that decision. Live core Apply blocks unresolved duplicates, including CLI and scheduled sync. Recovery uses the archive receipt; these archives do not use the workspace restore format.

## Structured alert settings

Local settings support optional `[alert]` with `kind = "system"`, `"pushover"` (`token`, `user` references), or `"ntfy"` (`url`, optional `token` reference). A reference is one of:

- `{kind: "gopass", entry}`: resolves `gopass show -o` for the named entry.
- `{kind: "keychain", service, account}`: resolves a generic password item from macOS Keychain.
- `{kind: "windows-credential", target}`: resolves a generic UTF-16 text password from Windows Credential Manager.
- `{kind: "environment", variable}`: reads the named environment variable.

Only references are serialized; credentials are read on each delivery. Store access failures propagate to CLI/scheduler errors and the corresponding Activity error ledger, remain immediately retryable, and never include provider stdout/stderr. The macOS helper includes standard Homebrew tool directories when resolving gopass. UI defaults come from the selected machine's `gopass` capability, then its native secret store. ntfy delivers push notifications to subscribers of the configured HTTPS topic.

v0.2.1 `token_env`/`user_env` fields are accepted as environment references and replaced with canonical reference tables on save. Custom commands continue to use `alert_command` argv with no shell.

Plans include `install_targets` with the renderer's harness, skill root, instructions file and project scope, plus `staging_root` when staging is active. Review groups by skill × agent × project; the badge counts these groups, unresolved legacy copies and otherwise unrepresented errors once each. File details expand inside their group. Apply names its write/removal count and shows the effective staging or real destinations.

`update_desktop({install})` returns `{message, available, installed}` from the native updater. Settings offers installation only after an available result and restart only after a successful installation; it does not infer state from the message text. A manual check records successful-check health but does not consume an automatic slot, open a catch-up hour, or persist pending installation for a later launch. It clears any older queued automatic update; the independent automatic schedule still applies.

### Updates without a workspace

`ratatoskr update --desktop` writes `desktop-update-request.json` in the app-data directory. The app polls for it every five seconds, writes its answer to `desktop-update-response.json` (`current`, `installing`, or `failed`), and records the final `installed` or `failed` outcome after restarting. Only the CLI writes the request and only the app writes the response. Each request names the desktop executable that must carry it out; other copies ignore it. The running app holds `desktop-instance.lock` and records its executable in `desktop-instance.json`. The CLI reads them to decide whether to start the app (directly, with `--background`) instead of going through the OS launcher, which would activate or reopen it. On macOS a second desktop process exits if the lock stays taken for five seconds; Windows keeps forwarding second launches through the single-instance plugin.

`read_update_settings` returns the effective `endpoint`, `override_active`, `automatic`, and `time`. `startup_update` and `update_desktop` work before a workspace is connected, using compiled official defaults. Existing configurations retain their update overrides. `read_local_settings` and `save_local_settings` remain workspace operations; onboarding Settings displays the default schedule and exposes manual check/install/restart without invoking those operations.

## Windows tray sync

`background_status()` returns null on macOS and a Windows status with `paused`, `interval_seconds`, `running`, `last_success`, `skills`, `login_enabled`, `error` and `migration_error`. Pause suppresses automatic runs; an explicit Sync now can still run once. `set_background_preferences({paused, intervalSeconds})` changes only non-null fields, preserving the other stored preference; `migrate_background()` requests another safe legacy-task migration. Owned-task migration enables hidden login startup before removal; failure to enable it preserves the task. `login_enabled` reflects the actual saved setting, and tray settings warn when it is off. On Windows `sync_workspace()` requests the same resident controller used by the tray menu; the response is a current snapshot, not a claim that the asynchronous run already finished.

`set_desktop_busy({busy})` communicates unsaved editors, operations, and pending navigation to the shared native updater on every desktop platform. `startup_update()` awaits the same once-only native startup decision on Windows and macOS before the frontend becomes interactive. The Rust scheduler runs without frontend timers and reevaluates the local clock every minute, including after sleep. Windows separately owns its sync timer in Rust; macOS sync still uses launchd.

The app-data `desktop-update.json` retains `last_check` (attempt), effective `settings`, and `pending`, and adds `last_success`, `monitoring_since`, and `catch_up_started`. A missed window is checked on the next tick, with a non-extending one-hour catch-up installation period. The configured nightly installation hour and startup catch-up remain supported. Checks and installation use the compiled signing trust root or explicit fork override, workspace locks, and a busy-state recheck after downloading. Two completed nightly windows without success set `desktop_update_error` through `desktop-update-notification.json` and the configured alert transport. The existing error/recovery deduplication applies; delivery failures remain retryable. Legacy state begins a grace period. On macOS monitoring uses the later of the last success and the persisted monitoring baseline, including time with the app closed. The launchd `sync` helper owns overdue alerts; the desktop never sends a second overdue alert. It reads `~/Library/Application Support/io.github.oana-ffg.ratatoskr-skill-sync/desktop-update.json` and keeps its migration grace period in `<state_dir>/<machine>/desktop-update-watchdog.json`, without writing desktop-owned state. Missing state for a bundled helper and pre-0.2.9 state without successful-check fields get one persisted grace period of two completed windows. A CLI-only install with no desktop state or prior monitoring is excluded. Corrupt/unreadable state is an alertable monitoring failure. Other desktop platforms retain the resident watchdog and startup-baseline behavior. The general `app-error` event carries a nullable error string for visible frontend diagnostics; Windows also folds update errors into tray status. `job-status` adds `background_error` for migration/runtime diagnostics. The in-process sync path skips standalone-helper self-update and delegates native system toasts to the tray controller, while retaining configured remote/custom alert transports.

Tray notification ledgers store stable problem identities separately from full messages. The shared, persisted problem-toast limit is one per 3600 seconds; repeated identities do not repeat after that interval. Recovery is delivered once for each notified problem that clears.

## macOS resident lifecycle and update notifications

Closing the main window hides it while the Rust update scheduler keeps running. A Dock reopen (`RunEvent::Reopen`) shows and focuses the same window. Application Quit/Cmd+Q remains unprevented. macOS and Windows share close/hide, show/focus, `--background` startup and one-shot restart visibility logic. The frontend uses the runtime OS to leave close-to-hide to native code even with unsaved edits, preserving the editor instead of offering to destroy its window. Windows retains its sync tray, migration, native toast and single-instance behavior; macOS sync remains a launchd helper job.

Lifecycle failures use `app-error` when the main window is visible and its frontend listener is ready. Before that listener attaches or while hidden, a native dialog reports the failure instead. The frontend acknowledges listener readiness through `set_app_error_listener({active})`; a page load resets it.

Open at login stays off until explicitly enabled. macOS login launches use `--background` and an initially invisible window. Starting a newer app refreshes an already enabled login entry with the background argument; it does not enable an opted-out entry. With automatic updates on and login startup off, Settings explains that automatic desktop updates require Ratatoskr to remain running. This warning uses `runtime_platform()` (the desktop process OS), independently of the selected machine profile.

Update reporters share the existing notification transport and per-ledger OS lock. New or never-delivered (`sent_at == 0`) errors are sent immediately when a route works; unchanged delivered errors repeat after 24 hours; a delivered problem's recovery is sent once. Skipped standalone checks retry pending errors/recoveries instead of clearing them. The macOS helper retries specific desktop-update errors and preserves them over generic overdue warnings. A successful desktop check clears desktop and bundled-helper update errors. Bundled helpers never check/install standalone updates during scheduled sync; the obsolete bundled-helper self-replacement error is cleared because that operation belongs to the desktop updater. Unknown helper errors remain retryable until a successful update check.

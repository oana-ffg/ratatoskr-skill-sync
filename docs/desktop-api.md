# Desktop command API (0.2.2)

PR #2 provides the Rust implementation, CLI equivalents and Tauri endpoints for the UI redesign. The desktop uses Svelte 5 and CodeMirror 6; structured edits use the same Rust core as the CLI. All command failures propagate as rejected invocations with contextual error text.

## Conflict review and recovery

- `preview_conflict({path, action})` returns a `ResolutionPreview`; actions are `copy-back`, `reinstall`, `adopt`. Copy-back is refused for variables, conditional comments, injected frontmatter, missing installed files and missing source mappings.
- `read_conflict_source({path})` returns the actual mapped source document, including selected variant files and global instructions. Binary files reject text editing.
- `preview_source_edit({path, text})` validates a manually edited source draft and returns an `edit-source` resolution preview.
- `resolve_conflict({expected})` accepts the **complete serialized JSON preview**, recomputes it under the operation lock, backs up actual installed bytes, missing-file state, original ownership and any affected source, and returns the backup directory. It rechecks again after backup. Do not reconstruct or edit preview fields in the client.
- `restore_workspace({backup})` verifies the existing backup format plus the optional source/ownership extensions, restoring the prior state. A source changed after the backed-up action is refused. Current conflicts must first be resolved. Restored hand edits reopen conflicts.

The snapshot plan retains human-readable `conflicts` for CLI compatibility and adds `conflict_details` containing `path`, `kind`, `diff`, mapped `source` and `copy_back_blocked`. Kinds are `hand-edited`, `removed`, `permission-change`, `unmanaged-collision`, and `unmanaged-directory`. Directory actions enumerate individual effects. Binary/large text diffs are null; effect hashes and executable state remain available. Adoption records ownership as-is; pending source differences are still replaced by a later reviewed Apply.

## Rendering and editor analysis

- `list_preview_targets()` returns enabled `{machine, harness}` pairs from all source profiles.
- `preview_skill({name, project, markdown, metadata, machine, harness})` renders an isolated draft with the deployment renderer. Returns resource-relative paths and text, with null text for binary output. Empty results mean the skill is disabled or excluded. Template context preserves the selected profile's values; destinations stay isolated even for a different OS/live profile.
- `analyze_skill({name, markdown, machine, harness})` returns `blocks`, `variables`, and `diagnostics` using the **same parser, variable resolver and frontmatter validator** as rendering. Every range is half-open `{from, to}` measured in **UTF-16 code units**, suitable for CodeMirror/JavaScript string offsets. Blocks include whole/opening/body/closing ranges, selector arrays, selected-context activity and reachability across enabled machine × harness contexts. Variables include their name, selected-context activity and resolved scalar value or null. Diagnostics include severity (`error` or `warning`), message and range.

Analysis diagnoses parser errors, unknown active variables, malformed substitutions, invalid YAML/identity/description and blocks matching no enabled target. It analyzes the supplied Markdown independently of `skill.toml` targeting; use `preview_skill` to validate metadata, variants and final selection. Frontmatter diagnostics use the frontmatter range; errors without a narrower source location use the document range. Neither analysis nor preview writes the real source or installed output.

## Agent definitions

- `list_agent_presets()` returns documented defaults with official source URLs.
- `preview_agent({target})` returns a source document with candidate `harnesses.toml` text and the original content hash. Existing text/comments are preserved, with the new definition appended. Duplicate tags reject.
- `add_agent_target({target, expectedHash})` validates and saves the definition using the expected source hash, returning a snapshot. Enable its tag in intended machine profiles separately.

`target` contains `tag`, `global_folder`, `project_folder`, `instructions_file`, and nullable `instructions_folder`. The last field supports Codex's separate instruction location. Preset scope is global skills/instructions plus project skills; this release does not generate project instructions. See [verified preset paths](agent-presets.md).

## Background work and login

- `read_job_status()` returns `last_run` (start/completion Unix seconds and error), `current_error` from the sync dedup ledger, separate updater errors, and bounded logs/history. A missing completion is explicit. All errors are persisted even without a notifier, and failed notification deliveries remain retryable.
- `detect_helper()` returns the executable bundled beside the desktop or an executable standalone helper found on absolute PATH entries. Missing helpers reject explicitly.
- `preview_schedule({seconds})` and `enable_schedule({seconds})` detect the helper themselves; clients cannot substitute another helper between preview and installation by submitting a path. `disable_schedule()` preserves existing schedule ownership checks.
- `login_enabled()` reads the Tauri autostart state. `set_login_enabled({enabled})` changes it explicitly and returns the verified state. Plugin registration never enables autostart by itself; default is off.

Existing `read_workspace`, source editing, configuration, project, sync/apply and update commands remain available. Nightly updates must continue to guard all unsaved editor/form state in the replacement UI.


## Structured desktop editing

- `editor_view({markdown, metadata})` returns description, UTF-16 frontmatter range and parsed metadata values. It does not rewrite source.
- `edit_skill_draft({markdown, metadata, edit})` returns draft Markdown/metadata only. `edit.kind` is `description` (`value`), `metadata` (`path`, nullable typed `value`), `only` (`from`, `to`, `selectors`, `existing`), `remove-only` (`from`), or `variable` (`from`, `to`, `name`). Conditions use the shared parser. All offsets are UTF-16 source offsets, before display newline normalization.
- `save_skill_draft({name, project, markdown, metadata, markdownHash, metadataHash})` validates both drafts together, checks both hashes, and saves only changed files under the operation lock. A second-file failure rolls back the first file when possible; process termination is not a two-file transaction.
- `create_desktop_skill({name, project, description})` constructs valid frontmatter in Rust. `rename_source_skill({name, project, newName, expectedHash})` explicitly renames the directory and identity.
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

`update_desktop({install})` returns `{message, available, installed}` from the native updater. Settings offers installation only after an available result and restart only after a successful installation; it does not infer state from the message text.

### Updates without a workspace

`read_update_settings` returns the effective `endpoint`, `override_active`, `automatic`, and `time`. `startup_update`, `nightly_update`, and `update_desktop` work before a workspace is connected, using compiled official defaults. Existing configurations retain their update overrides. `read_local_settings`, `save_local_settings`, and `save_update_schedule` remain workspace operations; onboarding Settings displays the default schedule and exposes manual check/install/restart without invoking those operations.

## Windows tray sync

`background_status()` returns null on macOS and a Windows status with `paused`, `interval_seconds`, `running`, `last_success`, `skills`, `error` and `migration_error`. `set_background_preferences({paused, intervalSeconds})` persists the pause/interval state; `migrate_background()` requests another safe legacy-task migration. On Windows `sync_workspace()` requests the same resident controller used by the tray menu; the response is a current snapshot, not a claim that the asynchronous run already finished.

`set_desktop_busy({busy})` communicates unsaved/busy UI state to the native Windows nightly updater. Windows owns sync and update timers in Rust; macOS retains the frontend update policy. `job-status` adds `background_error` for migration/runtime diagnostics. The in-process sync path skips standalone-helper self-update and delegates native system toasts to the tray controller, while retaining configured remote/custom alert transports.

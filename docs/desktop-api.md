# Desktop command API (0.2.0)

PR #2 provides the Rust implementation, CLI equivalents and Tauri endpoints for the UI redesign. The vanilla TypeScript interface is a temporary command exercise surface; the Svelte/CodeMirror redesign is a separate PR. All command failures propagate as rejected invocations with contextual error text.

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

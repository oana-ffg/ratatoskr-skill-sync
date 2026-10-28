# Content format

Ratatoskr reads the existing agentfiles layout:

- `harnesses.toml`: a table for each agent tag, containing portable relative `skills_dir`, relative `global_file`, optional `project_root`, optional `global_root` for a separately located global instructions directory, and `roots` mapping OS names to absolute root templates.
- `machines/`: one TOML file per machine identifier. Required fields are `os` and a nonempty `harnesses` list. `mode` defaults to `staging`; `live` also requires a verified `home`. Optional `paths` and `capabilities` provide template values and skill selection.
- `skills/`: one directory per portable skill name. Each contains `SKILL.md` with YAML `name` matching the directory and a nonempty `description`, plus required `skill.toml`.
- `global/AGENTS.md`: optional shared global instructions, rendered to each target's configured global filename.

The initial reader uses machine `repo` and `alert_command` fields only as available template context. It does not execute a legacy alert command or move its checkout according to `repo`. The local Ratatoskr configuration owns source/state paths. Scheduled syncing and alerts use the local configuration; they do not execute legacy machine-profile alert commands.

## Validate before deployment

Run `ratatoskr validate <source-path>` (optionally `--json`) to render all machine profiles and their enabled harnesses without configuration or writes. Selection and disabled skills behave exactly as sync: excluded skills are counted as skipped; selected skills, matching variants and global instructions use the shared renderer. Live root templates are checked for the profile's OS without accessing foreign destinations. Every independent render failure is reported with its source path and machine/harness/skill context. Invalid configuration for one profile does not prevent the others from being checked. Fix blocking profile/skill metadata errors and rerun to inspect content that depends on those definitions.

Problem paths in human output and JSON `path` fields are relative to the source checkout and use forward slashes on every OS. Problems with the same path and message are grouped with all affected contexts.

The command does not register projects or inspect Git, local ownership, host permissions, or language-specific syntax. No profiles is an inspection error rather than a successful empty validation. Use `plan` on the actual target before deployment.

## Selection and templates

Set `enabled = false` at the top level of `skill.toml` to disable a skill without deleting its source. It can also contain `[target]` arrays named `harnesses`, `os`, `machines`, and `capabilities`. Members of each ordinary list are OR; different keys are AND. All capabilities must be present. An empty ordinary selector excludes the skill. Unknown selector keys are errors. Unknown values that no defined harness, machine profile, or operating system can match are `ratatoskr validate` warnings and do not change its exit code. The scan reports a problem when a present file or directory cannot be read, traversed, or parsed. A missing `global/AGENTS.md`, `skills/` directory, or `variants/` directory is not one of those problems.

Markdown supports `only` HTML comment blocks: an opening comment starts with `only`, followed by `harness=codex`, `os=windows`, or `machine=brunnr`, and a closing comment contains `/only`. Comma-separated values mean OR and separate keys mean AND. Blocks cannot nest, even in excluded content, and are interpreted inside Markdown code fences too.

Descriptions can contain inline `only` blocks too. For example, `description: "Review code.<!-- only harness=codex --> Include Codex guidance.<!-- /only -->"` renders the extra sentence only for Codex. The desktop description’s **Only for…** control adds conditional text without editing template syntax; each section can be edited or made shared with **Remove condition**. Live preview shows the selected agent’s rendered description. The 1–1024 character limit applies to each rendered description, excluding template markers.

All UTF-8 text supports scalar variables with `{{af:...}}`, including `home`, `machine`, `harness`, and dotted values under `paths`. Undefined variables in included content fail. Substitution is literal: quote correctly for the destination language. Opaque binary files are copied unchanged. Markdown, TOML, Python and `.txt` files require UTF-8.

`variants/harness=codex/`, `variants/os=windows/`, and `variants/machine=brunnr/` add or replace files relative to the skill root. Matching variants cannot both replace the same path. Variants cannot replace `skill.toml`. `[frontmatter.<harness>]` adds YAML fields but cannot replace shared fields or skill identity.

Portable filenames exclude traversal, Windows device names, invalid Windows characters, trailing dots/spaces, and case collisions. Unix executable bits are preserved; Windows does not use Unix executable permission bits. Known runtime artifacts (`__pycache__`, `.pyc`, `.DS_Store`) are ignored, but symlinks are always refused.

## Local project marker

`.ratatoskr.toml` contains a `source` string, a portable path relative to the Git root. The local registry stores only explicitly registered project roots. Ratatoskr monitors this source and marker for Git changes and renders its `skills/` content using shared global harness/machine settings. Live outputs use `project_root` (default `.<harness>`) and `skills_dir` inside the project. Staging outputs use a project-path hash beneath Ratatoskr state. A project source does not need its own harness or machine files. Global instructions are not rendered into projects.


## Agent target form

The desktop preset/custom form previews a complete `harnesses.toml` update and saves it with the source hash checked. It creates a definition, without changing machine selection. Add its tag to each intended profile's `harnesses` array using the source editor. A definition contains `skills_dir = "skills"`, the chosen `global_file`, `project_root`, and per-OS `roots`. Codex additionally uses `global_root = "{{af:home}}/.codex"` while its skill root is `{{af:home}}/.agents`. Staging keeps both under the isolated harness staging directory. Existing definitions are not migrated automatically.

The form accepts an absolute global folder or a `{{af:home}}` template, a portable project subfolder, an instructions filename, and an optional separate global instructions folder. Power-users can keep different per-OS roots in text. Presets and their official references are listed in [agent-presets.md](agent-presets.md). `ratatoskr agent presets` emits the same definitions; `agent preview --input` accepts a JSON target and returns candidate source text with its original hash. `agent add --input --expected-hash` saves it after validation.

Project rendering remains skill-only. Instructions filenames and a separate global instruction root apply to global installations; project instruction files are not generated by this release.

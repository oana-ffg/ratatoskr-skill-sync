# Verified agent presets

Official documentation checked on 2026-10-16, with Antigravity checked on 2026-11-19. The same URLs are attached to definitions in `crates/ratatoskr-core/src/presets.rs` and displayed in the picker. Paths below are documented defaults; environment overrides require editing the source definition. `~` denotes the profile's home; source roots use `{{af:home}}`.

| Agent | Global skills | Project skills | Global instructions | Official sources |
| --- | --- | --- | --- | --- |
| Claude Code | `~/.claude/skills` | `.claude/skills` | `~/.claude/CLAUDE.md` | [Skills](https://code.claude.com/docs/en/skills), [Memory](https://code.claude.com/docs/en/memory) |
| Codex | `~/.agents/skills` | `.agents/skills` | `~/.codex/AGENTS.md` | [Skills](https://developers.openai.com/codex/skills/), [Instructions](https://developers.openai.com/codex/guides/agents-md/) |
| Gemini CLI | `~/.gemini/skills` | `.gemini/skills` | `~/.gemini/GEMINI.md` | [Skills](https://geminicli.com/docs/cli/skills/), [Context](https://geminicli.com/docs/cli/gemini-md/) |
| GitHub Copilot CLI | `~/.copilot/skills` | `.github/skills` | `~/.copilot/copilot-instructions.md` | [Skills](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-skills), [Instructions](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-custom-instructions) |
| OpenCode | `~/.config/opencode/skills` | `.opencode/skills` | `~/.config/opencode/AGENTS.md` | [Skills](https://opencode.ai/docs/skills/), [Rules](https://opencode.ai/docs/rules/) |
| Antigravity | `~/.gemini/config/skills` | `.agents/skills` | `~/.gemini/config/AGENTS.md` | [Skills](https://antigravity.google/docs/skills), [Rules](https://antigravity.google/docs/rules) |

Codex uses `CODEX_HOME` for an overridden instructions root. OpenCode users with a nondefault configuration root should edit the definition. Compatible alternate skill discovery directories are intentionally not separate presets, avoiding duplicate installations. New content uses the current Claude/Codex presets; existing source definitions retain their configured paths.

Antigravity's global skills are shared by its App, CLI and IDE. It shares `~/.gemini` with the Gemini CLI preset: their global roots overlap, so Ratatoskr refuses to enable both on one live machine. Antigravity and Codex also share project `.agents/skills`. When enabling both, set `project_harnesses = ["codex"]` (or choose Antigravity alone for projects); both agents can read the one project installation.

Antigravity hosts launching `antigravity acp --gemini-dir <dir>` read global rules from `<dir>/AGENTS.md` and `<dir>/config/AGENTS.md` instead of `~/.gemini`; adjust the source definition for such a host. The preset uses the standalone `config/AGENTS.md` file without frontmatter, rather than modular `config/rules/*.md`, which require frontmatter. Each rules file is capped at 24,000 bytes and truncated beyond that, with an aggregate 20,000-token rules budget. These paths and the host override were also tested empirically with the Antigravity CLI on macOS on 2026-11-19.

## Omitted presets

- **Cursor:** [Skills](https://cursor.com/help/customization/skills) verifies global `~/.cursor/skills` and project `.cursor/skills`. The [Rules reference](https://cursor.com/docs/rules) describes global User Rules configured in the app and project `.cursor/rules/*.mdc`. A default plain global instructions file compatible with this renderer was not verified, so a complete preset is omitted.
- **Grok CLI / Grok Build:** [Skills documentation](https://docs.x.ai/build/features/skills-plugins-marketplaces) verifies global `~/.grok/skills`, project `.grok/skills`, repository `AGENTS.md` discovery and Claude compatibility. A native default global instructions file was not established from that documentation, so a complete preset is omitted.

The form can define custom targets when a user has verified their own configuration. Tests validate every included definition, template/path rejection, duplicate-tag refusal, staging rendering, and the independent global instruction root. Except for the Antigravity CLI check above, the individual agent applications were not launched to test discovery.

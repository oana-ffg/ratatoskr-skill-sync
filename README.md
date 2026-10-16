# Ratatoskr Skill Sync

<img src="desktop/public/ratatoskr.png" width="130" align="right" alt="Ratatoskr, a red squirrel holding an acorn">

One Git source for your AI-agent skills and instructions, adapted to each machine, operating system, and harness.

Ratatoskr is a Rust command-line helper with an optional Tauri 2 desktop app. The helper runs with the window closed. Your content lives in a separate repository, which can be private. The tool is MIT licensed.

## What the first draft does

- Render machine, OS, harness, and capability conditions; substitute variables; select file variants; add harness-specific frontmatter.
- Preview output diffs and refuse an apply if an owned file was hand-edited, removed, or changed permissions. Existing destinations require explicit adoption with verified backups.
- Create, clone, or connect a content repository. Create, edit, enable, disable, and archive source skills. Validate syntax before saving and detect stale editor versions.
- Configure Claude, Codex, or custom agent targets through editable source definitions and machine profiles.
- Render global and registered project skills. Report uncommitted and unpushed project skill-source work.
- Install or remove a per-user launchd or Windows Task Scheduler job. Each run fast-forwards a clean content checkout, applies it, and exits. Optional command-based failure/recovery alerts are deduplicated.
- Check and install cryptographically signed helper and desktop updates, with nightly automatic updates enabled by default. A release workflow builds signed draft releases for Apple Silicon, Intel Mac, and Windows x64.

**Development draft:** no release binaries have been published. Apple/Microsoft publisher signing is not configured. Start in staging and review the [verification record and remaining limits](docs/verification.md).

## Build and run

Install stable Rust (1.90 or newer), Git, Node.js 22.18 or newer, and [Tauri's platform prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
cargo build --locked
cargo run -- --help
cd desktop
npm ci
npm run tauri dev
```

To package a native app, run `npm run tauri build` inside `desktop`. The build prepares and bundles the independent helper. The browser-only `npm run dev` preview has no filesystem access.

The executable is `ratatoskr`. `init --create` creates a Git content repository with a profile for the current host, staging enabled, and Claude/Codex presets. Choose an absolute source directory and a lowercase machine identifier. `init` without `--create` connects an existing checkout; `--remote` clones a Git URL into a new directory. The selected machine must exist under `machines/`. Initialization never applies outputs.

After connecting:

```sh
ratatoskr plan
ratatoskr apply
ratatoskr status
```

Use `--config` to select a local configuration or set the absolute `RATATOSKR_CONFIG` environment variable. Both the desktop app and helper respect it. `--json` emits structured plans and status. `status` returns 1 for pending changes, conflicts, project inspection problems, or uncommitted/unpushed skill work; operational errors return 2.

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

`ratatoskr schedule plan --helper` previews an OS job for the supplied absolute executable path. `schedule install` registers it; `schedule remove` removes only the recorded, unchanged job. `--seconds` accepts 60–86400 (default 60). The desktop app exposes the same operations. No resident watcher is installed. Pending project skill commits or pushes also return a sync error and use the configured alert command.

`alert_command` in local configuration is an argv array. Ratatoskr appends a plain-English message and executes it without a shell. New failures alert immediately, unchanged failures repeat after 24 hours, and recovery alerts once. Failed delivery remains an error. Without an alert command, failures still return nonzero and appear in scheduler logs.

`ratatoskr adopt` backs up existing matching target directories and global files with corresponding source content before recording ownership. It does not write generated output. Review `plan` before applying: adopted files absent from source are scheduled for removal. `ratatoskr restore` accepts the backup directory printed by adoption, verifies every stored hash, and refuses current hand edits or unmanaged overwrite. Archiving a source skill moves it into `.ratatoskr-archive` on the same volume.

## Updates and operating limits

Official GitHub Releases and the trusted public signing key are built into both applications. No per-machine key setup is needed. Automatic updates default to **00:00 local time**; change the time or switch them off in the desktop Signed updates panel or local TOML:

```toml
[updates]
automatic = true
time = "00:00"
```

The running desktop checks once during the one-hour nightly window. An available update waits for unsaved editors and apply/sync/schedule work to finish, then installs and relaunches the connected workspace. No available update means no restart. If work stays busy until the window closes, it tries the next night or at app launch. Launch catches up on a missed nightly window, installing and relaunching before the workspace becomes interactive. Checks are persisted across launches. The standalone helper records its last attempt and checks at most once per local day, at or after the chosen time, before that run's sync. Its next run uses the new binary. Update failures have separate deduplicated alerts and never prevent content syncing. Bundled helpers update with the desktop.

Forks may explicitly set `updates.endpoint` and/or `updates.public_key`. The UI prominently labels active overrides. See [release operations](docs/releases.md) for the signing trust root and GitHub secrets. Cryptographic update signatures are separate from Apple notarization and Windows publisher signing.

The ownership manifest belongs to one source and machine. Symlinks, traversal, case collisions, overlapping roots, conflicting variants, and malformed source files fail validation. There is no force-overwrite option. An OS lock serializes Ratatoskr applies; the desktop additionally refuses a plan that changed after review.

Each file and manifest replacement is atomic; the entire output tree is not a transaction. An I/O error midway can leave partial changes, which the old manifest exposes as conflicts. Other editors can still race the helper. Backups are not automatic snapshots of every sync.

Project Git status covers its registered source and marker, using local upstream references without fetching. A missing upstream stays visible. `sync` contacts the content upstream, but never resets, stashes, rebases, or force-pushes.

[Architecture](docs/design.md) · [Branding](docs/branding.md) · [Contribution rules](AGENTS.md)

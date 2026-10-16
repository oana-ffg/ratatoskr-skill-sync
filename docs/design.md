# Architecture

Ratatoskr keeps tool code separate from skill content. The content repository can be private; this public MIT repository contains no personal skills or machine profiles. The design was implemented independently without reading competitor code.

## Components

- `ratatoskr-core` parses and renders content, plans and applies owned files, manages source edits and adoption backups, inspects Git, generates OS schedules, reports failure/recovery alerts, and verifies helper updates.
- `ratatoskr-skill-sync` provides the `ratatoskr` one-shot CLI. An OS scheduler invokes `sync`; no desktop process or resident watcher is required.
- `ratatoskr-desktop` provides Tauri commands around the same core. The TypeScript/Vite UI edits local content, reviews diffs, configures schedules, and invokes the official Tauri updater. The helper is bundled as a sidecar.
- A content checkout provides portable sources and target definitions. Git credentials use the existing Git authentication mechanism; Ratatoskr stores no Git tokens.
- Local configuration/state holds source selection, project registry, manifests, OS locks, scheduler records, notification state, and adoption backups.

## Rendering and ownership

Global and project content share the selected machine and harness definitions. Conditions, variables, file variants, and harness frontmatter are resolved into an immutable desired-file map. Projects provide only skill content and a marker; they do not vendor the rendering tool.

Planning compares desired bytes and executable permissions against both live destinations and the ownership manifest. Applying checks all destinations before writing, rechecks each file, atomically replaces individual files, and writes the new manifest last. The UI submits the exact reviewed plan; the core compares it again under the apply lock.

Adoption backs up existing files and records their current fingerprints without rendering over them. Restore validates backup hashes and current ownership before writing. A source archive uses a same-volume rename; no recursive source deletion occurs. Unregistering a project detaches its owned output and leaves the project files intact.

The app keeps configuration and project-removal controls available when source preview fails. The failure remains visible and blocks apply. CLI failures exit nonzero.

## Background operation

A dedicated clean deployment checkout avoids scheduled pulls colliding with authoring edits. `sync` holds an OS-backed lock, performs a fast-forward-only pull, applies, handles optional standalone-helper updates, then reports success or failure through a deduplicated notification ledger. It retries apply on every invocation, even if Git has not changed. There is no watcher or per-project CI requirement.

macOS uses launchd StartInterval and RunAtLoad. Windows uses Task Scheduler repetition and logon triggers. The helper requires an available content volume, state directory, Git credentials, and PATH. Schedules belong to the current user and are removed only when their definition still matches the stored record.

## Updates

The independent CLI verifies a Minisign signature, then verifies the signed executable's reported version against the manifest before self-replacement. The native app uses the official Tauri updater and replaces the complete bundle, including its sidecar. A bundled helper never replaces itself independently. The official HTTPS release endpoint and public key are compiled into both applications. Explicit local fork overrides are visibly labelled. Nightly automatic updates default to midnight local time; the desktop defers installation until editors and operations are idle within a one-hour window, then relaunches only after installation. The one-shot helper records its last daily attempt and updates before sync. Sync first verifies the configured deployment branch (default `main`) and its remote upstream before any update, pull, or apply.

## Limits and review focus

A multi-file apply is not a filesystem transaction, and OS locks cannot stop unrelated editors. Source paths are trusted configuration but traversal, symlinks, collisions and ownership boundaries are checked. Source edits use hashes to detect stale editor state; a concurrent writer can still race the final file replacement. Project status uses the last local upstream reference.

Initial development focuses on macOS and Windows. Linux core checks are included, but Linux desktop packaging and scheduler installation are outside this draft. See the verification record for what has actually run.

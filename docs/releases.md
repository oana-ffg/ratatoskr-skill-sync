# Releases and signed updates

The official trust root is compiled into the helper (`crates/ratatoskr-core/src/update-public-key.txt`) and the desktop updater configuration. Both use:

`https://github.com/oana-ffg/ratatoskr-skill-sync/releases/latest/download/latest.json`

`scripts/release-manifest.mjs` publishes that manifest with all seven desktop/helper platform entries. Users do not configure signing keys. No public release is created by this change.

## Signing and release operations

The official key pair was generated with the Tauri signer. Its private key and password are stored only in gopass and GitHub Actions secrets:

- `ratatoskr/release/tauri-signing-key` → `TAURI_SIGNING_PRIVATE_KEY`
- `ratatoskr/release/tauri-signing-key-password` → `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

Load each value directly from gopass into `gh secret set` over stdin. Never print secrets or write them to local files. The release build reads the committed public key from `tauri.conf.json`; no repository public-key variable is required. A release build verifies that the private and public keys agree.

1. Set matching versions in workspace `Cargo.toml`, `desktop/package.json`, and `desktop/src-tauri/tauri.conf.json`; refresh the lockfiles. Version changes require the normal PR, green CI, and approval from an agent on another harness. The author merges after approval unless Oana grants a specific bypass.
2. Dispatch **Build signed draft release**. It builds Apple Silicon, Intel Mac, and Windows x64 desktop bundles and standalone helper executables. The bundled helper is prepared from the same source version.
3. Download and test artifacts on actual Macs and Windows before publishing the draft. Inspect `latest.json`, including all seven platforms, and test an older installed version updating to the candidate. Publish only after approval; the workflow itself leaves the release as a draft.

The workflow signs artifacts using Tauri/Minisign. Windows uses the signed NSIS installer directly; macOS uses signed `.app.tar.gz` updater bundles. Apple Developer ID signing/notarization and Microsoft publisher signing remain separate, unconfigured integrations. An update signature authenticates the update without establishing an OS publisher identity.

## Windows helper pair

Windows packaging enables Cargo's `windows-helper` feature and builds both the normal console CLI (`ratatoskr.exe`, PE subsystem 3) and the scheduled-sync helper (`ratatoskrw.exe`, PE subsystem 2). `prepare:helper` stages both sidecars, and the Windows-only Tauri configuration bundles both beside the desktop executable. macOS packaging still stages only `ratatoskr`.

The release publishes `ratatoskr-windows-x86_64.exe` and `ratatoskrw-windows-x86_64.exe` with independent signatures. The manifest requires `windows-x86_64-cli` and `windows-x86_64-windowless`. A standalone Windows update downloads, verifies signatures and checks matching versions for both before replacing either. The companion is replaced first, then the running helper uses the existing self-replacement mechanism; a self-replacement failure restores both original paths and returns an update error. A busy companion causes an explicit failure before self-replacement. Desktop updates replace both through the signed installer.

After installing the pair, run `ratatoskr schedule install` to migrate an existing owned task to the windowless command. Existing schedule labels/paths and removal's byte checks remain intact; edited or unowned definitions require manual review. The old v0.2.6 updater cannot acquire a companion it did not know about, so an upgrade initiated by that old standalone CLI must also download the windowless asset beside it (or use the Windows installer).

## Local update settings

An omitted `[updates]` table uses official defaults: automatic updates enabled at `00:00` local time. Set `automatic = false` to disable nightly installation, or set `time` to a 24-hour `HH:MM` value. The desktop panel edits these same settings when a workspace is connected. Without a workspace config, the app uses the compiled official endpoint/key and the automatic `00:00` schedule. Settings → Updates is available from onboarding for manual checks, installation, and restart. Connecting a workspace enables its existing update settings, including explicit overrides.

Forks may explicitly override `endpoint` (HTTPS without embedded credentials) and/or `public_key`. Omitted fields retain their compiled official values; empty or invalid explicit values fail validation. The desktop shows **Custom update override active** and the effective endpoint whenever either field is present. Public keys accept Tauri's base64-encoded public-key file or a raw Minisign public-key line. A manifest cannot replace the trusted key. Key rotation requires an explicit trust migration.

`ratatoskr update` checks manually; `update --install` installs. Desktop manual controls remain available. Before the first published manifest, GitHub may return 404: the helper treats that as no release, while the Tauri desktop updater reports a missing release manifest through the normal error/alert path.

## Nightly behavior

The running desktop checks during the first hour after the configured time. The window spans midnight when needed. Polling every 60 seconds evaluates local state, including during onboarding; attempts are saved in `<app_data_dir>/desktop-update.json` before networking, so reopening does not repeat a completed check in the same nightly window. When that app ledger is absent, an existing workspace ledger at `<state_dir>/<machine>/desktop-update.json` is read for continuity; the next attempt writes to the app directory. This keeps completed checks and deferred updates across the 0.2.0 transition.

At launch, a missed window triggers a catch-up check before the workspace becomes interactive. An available signed update is installed and the app relaunches before editing starts. Startup waits for other helper apply/sync/schedule work to finish when a workspace exists. Without a workspace, installation needs no workspace locks. A previously available update deferred by busy work is checked again on launch; changed update settings also trigger a fresh check.

During an interactive session, a cached available update waits while editors contain unsaved configuration, skill, source, or new-skill work, or while an app operation is busy. Installation rechecks the saved configuration and takes the same cross-process locks as helper sync, apply, and schedule changes. Editors are frozen through installation/relaunch only after they are idle. The connected workspace comes from its existing persisted local configuration. No update means no installation or restart. Work still busy at the end of the hour defers until the next night or app launch. A failed check alerts and retries the following night; an idle tick or reopening after that failed check does not hammer the endpoint. Startup errors are shown when the workspace becomes interactive.

The standalone helper stores its attempt timestamp in `<state_dir>/<machine>/helper-update.json` before networking, including failed attempts. It checks at most once per local calendar day, at or after the configured time. A signed update is installed before that run's content sync; the running process finishes its sync and the next scheduled process uses the replacement. A helper in a Mac app or beside the Windows desktop executable is updated only through the desktop bundle (`standalone_helper`).

Helper update failures never prevent the content pull/apply from running. Helper updates, desktop updates, and sync each use their own deduplicated notification state. An update failure and a sync failure can both alert during the same invocation. Skipped update checks and successful syncs cannot clear update alerts; a successful update check reports recovery. Failed update-alert delivery also leaves the sync running. The CLI still exits with an error for a failed update or notification, preserving all simultaneous failures in its output. Without `alert_command`, failures remain visible in CLI/scheduler output or the desktop status message.

Standalone installation enforces HTTPS, size limits, a valid Minisign signature, a newer semantic version, and agreement between the signed binary's `--version` output and the manifest. Tauri enforces desktop signatures and matching signed versions.

## 0.2.1 patch

Fixes desktop updates for fresh installs and apps without a connected workspace. Startup catch-up, nightly checks, and manual Settings checks/installations share the same app-level settings resolver. Desktop attempt/deferral state belongs to the app data directory; helper update state remains workspace-specific. Existing configured update overrides and idle guards are preserved. Invalid configurations and corrupt update state remain visible errors.

The version is bumped in the workspace, desktop package, native bundle config, and lockfiles. This PR does not build or publish a release.

## 0.2.2 patch

Fixes the structured editor's hidden-frontmatter layout and limits line numbers to Markdown source mode. Removes decorative copy from every app view. Review changes groups files by skill and agent, counts attention items, names concrete staging/live destinations, and separates legacy Codex choices. Alert credentials use references to gopass, macOS Keychain, Windows Credential Manager or environment variables; v0.2.1 environment-name settings remain readable and migrate on save.

The version is bumped in the workspace, desktop package, native bundle config, and lockfiles. Release preparation still uses the signed draft-release workflow. No release is published by this PR. Brunnr's first real v0.2.1 → v0.2.2 overnight installation is an acceptance check after approval and publication.

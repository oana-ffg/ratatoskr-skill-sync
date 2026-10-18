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

## Windows tray migration

Windows now ships the tray-resident desktop app and the console CLI. The `ratatoskrw` Cargo target, feature, packaging, release asset, paired-update logic and checks have been removed. The six-platform manifest contains desktop and CLI assets for macOS Apple Silicon, macOS Intel and Windows x64.

The signed NSIS desktop updater used by v0.2.7 still consumes the same `windows-x86_64` installer asset. The updater passes `/UPDATE`, and [Tauri CLI 2.12.3's NSIS template](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.3/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi#L298) skips the old uninstaller in that mode. Therefore an existing `ratatoskrw.exe` is expected to remain after an in-app upgrade. The new bundle has no previous-version resource-ownership manifest with which to safely identify that file; we do not delete a sibling executable by filename alone. The leftover is inert after owned-task migration, and the app never launches it. A normal interactive upgrade that runs the old uninstaller can remove the old sidecar, but the in-app update path does not depend on that.

A v0.2.7 **standalone CLI** updater requires both old helper assets and therefore refuses the new manifest before replacing either binary. That old client cannot be changed by a new manifest. Upgrade that installation with the signed desktop installer, or download and verify the new standalone CLI manually. New standalone CLI versions self-update only the CLI. No compatibility executable or unsigned alias is published.

At desktop startup, legacy task deletion requires the existing ownership record and exact local XML bytes, plus the task label derived from the configuration path and the expected state-file path. Both historical UTF-8 and UTF-16LE+BOM definitions remain recognized. Disabled tasks are deleted through the same path as enabled tasks. Migration first enables hidden Open Ratatoskr at login through the per-user Run key, then removes the owned task and records the replacement logon startup in the ledger. A login-start failure preserves the old task. Migration preserves the configured interval and enables tray sync; edited/unowned tasks are left untouched and block tray sync, with an error in the UI and job-status ledger.

Run-key autostart retains the existing preference when there is no owned task to migrate. Owned-task migration enables it to preserve the previous logon trigger. Tray settings warn when login startup is disabled. An already enabled entry is refreshed to include `--background`. Hidden windows are created hidden, avoiding a startup flash. A one-shot restart-state file in app data carries current window visibility across NSIS updater relaunch and explicit restart; foreground reopening overrides stale background launch arguments. Notification identities use stable operation contexts and typed error categories, independently for sync, migration, updates and app health. Full diagnostics remain in the UI and ledger. A persisted app-wide cap allows at most one problem toast per hour across all categories; a pending distinct problem can notify once the hour expires, and recovery remains allowed. They remain in memory during ledger-write failures; pending saves retry without repeating delivered toasts. Migration checks run at startup and when background settings open, with an explicit retry for failures. Native Windows timers preserve the nightly update window; unsaved desktop edits and core operation locks defer installation.

CI registers only harmless, far-future `cmd.exe /c exit 0` fixtures. It tests owned enabled/disabled migration, old XML encodings, edited-byte refusal, preservation of an unowned task, idempotence, and refusal of `schedule install`. The same integration executable runs under a disposable non-administrator account, with task/account/profile/fixture cleanup. Native desktop builds and NSIS packaging remain required CI checks; no release is produced by this change.

# Releases and signed updates

## v0.2.15 release notes

- Show the actual number of skills on each project card, independently of uncommitted files and unpushed commits. Clean projects say “No uncommitted or unpushed changes”. Counts include disabled skills.
- Make the library heading count follow the selected project and search filter, matching “View skills”. Report an unreadable skill list as unavailable instead of displaying a false zero.
- Extend Chromium/WebKit UI regression tests to cover project totals, scoped browsing, search counts, and failed skill reads.

## v0.2.14 release notes

- Defer workspace refresh quietly during editing. Ordinary edits no longer trigger a red freshness warning; actual refresh failures still show the reason and Retry.
- Save the current draft through the Rust core's validation even while live preview is pending. Failed saves show the actual reason and retain the draft; retrying clears the previous save error.
- Show preview installation details only when the rendered skill applies to the selected agent and machine.
- Add an editor UI CI job in Chromium and WebKit covering delayed previews, long edits, invalid-save recovery, source conflicts, target selection, and conditional description editing. Saves exercise Rust validation and filesystem writes in disposable public fixtures; failures retain screenshots and traces.

## v0.2.13 release notes

- Add **Only for…** controls to skill descriptions. Edit shared and conditional text, change or remove conditions, and inspect each agent’s rendered description and character count in live preview.
- Validate description length after rendering, so conditional templates can exceed 1024 source characters when each target stays within the limit. Saving also checks for new failures of the edited skill across all machine profiles, including project skills, while allowing incremental repairs of existing remote errors.

## v0.2.12 security patch

- Preserve existing Unix file modes and group ownership during updates and restores; refuse native ACL and other protection cases that cannot be preserved safely. This includes macOS. Old backups without protection metadata restore with owner-only permissions. Windows security-descriptor preservation remains separate work.
- Require HTTPS throughout ntfy and Pushover notification requests, including every redirect. HTTPS redirects remain supported; a downgrade fails visibly and remains eligible for retry.

## v0.2.11 security patch

Update `rustls-webpki` to 0.103.16 and `webpki-roots` / `webpki-root-certs` to 1.0.10. These releases distrust the compromised eMudhra intermediate CA keys and update the bundled trust roots used by helper and desktop HTTPS clients. Signed-update verification and existing update settings are unchanged.

## v0.2.10 release notes

- Refresh the desktop workspace snapshot when the window is shown or focused and every 60 seconds while visible. An "As of" indicator dates the last successful snapshot; refresh failures remain visible with Retry. Refresh waits while edits, dialogs or operations are in progress, preserving the editor and current selection. Window and updater errors share the app's error reporting.
- **On macOS, closing the window now hides it and keeps the app running. Use Cmd+Q to quit; click the Dock icon to reopen the window.** Open at login launches the app hidden with `--background`, updates existing enabled login entries, and preserves opt-outs. Settings warns when automatic updates are enabled but Open at login is off.
- On macOS, the launchd sync helper now reports overdue desktop update checks after two completed nightly windows without a successful check, even when the desktop app is not running. The helper must remain running and have a working alert route. Monitoring survives app restarts; missing or legacy desktop state gets a grace period, CLI-only installs without desktop monitoring are excluded, and unreadable monitoring state is reported as an error.
- Retry alerts that have never been delivered when delivery becomes possible. A shared delivery lock prevents concurrent reporters from sending the same transition twice; delivered errors retain their 24-hour repeat interval and recovery is sent once.
- Clear the obsolete bundled-helper self-update error. Bundled helpers leave their replacement to the desktop updater and no longer record that error; other pending errors remain retryable.
- Add `ratatoskr init --state-dir` for connect, create and clone. Without an override, a custom `--config` or `RATATOSKR_CONFIG` now uses an absolute state directory beside that config, named by appending `.state` to its filename (`config.toml` uses `config.toml.state`) instead of the live default state directory. The platform-default config keeps its existing default state location, and existing configs retain their saved `state_dir`. Separate state does not redirect a machine profile's sync destinations.

Still unverified: real Dock reopen, Cmd+Q and login-launch acceptance on the Macs, and native Windows tray show/hide acceptance.

## v0.2.9 release notes

- Add `ratatoskr validate <source>` to check content across machine profiles and enabled harnesses without local configuration or writing rendered files. It reports rendering problems with their machine, harness, skill and file context.
- Fix desktops never catching up after a missed nightly update window (for example, a Mac asleep at midnight), which left macOS apps on old versions without any error. Update checks now run on a shared native Rust scheduler instead of a WebView timer, catch up after a missed window, and alert when two completed windows pass without a successful check.
- **Automatic updates can now install and relaunch during the day after a missed nightly window.** Catch-up opens one persisted hour for installation once unsaved edits and running operations are clear. If the app stays busy until that hour expires, installation waits for the next night or app launch.

The official trust root is compiled into the helper (`crates/ratatoskr-core/src/update-public-key.txt`) and the desktop updater configuration. Both use:

`https://github.com/oana-ffg/ratatoskr-skill-sync/releases/latest/download/latest.json`

`scripts/release-manifest.mjs` publishes that manifest with all six desktop/helper platform entries. Users do not configure signing keys. No public release is created by this change.

## Signing and release operations

The official key pair was generated with the Tauri signer. Its private key and password are stored only in gopass and GitHub Actions secrets:

- `ratatoskr/release/tauri-signing-key` → `TAURI_SIGNING_PRIVATE_KEY`
- `ratatoskr/release/tauri-signing-key-password` → `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

Load each value directly from gopass into `gh secret set` over stdin. Never print secrets or write them to local files. The release build reads the committed public key from `tauri.conf.json`; no repository public-key variable is required. A release build verifies that the private and public keys agree.

1. Set matching versions in workspace `Cargo.toml`, `desktop/package.json`, and `desktop/src-tauri/tauri.conf.json`; refresh the lockfiles. Version changes require the normal PR, green CI, and approval from an agent on another harness. The author merges after approval unless Oana grants a specific bypass.
2. Dispatch **Build signed draft release**. It builds Apple Silicon, Intel Mac, and Windows x64 desktop bundles and standalone helper executables. The bundled helper is prepared from the same source version.
3. Download and test artifacts on actual Macs and Windows before publishing the draft. Inspect `latest.json`, including all six platforms, and test an older installed version updating to the candidate. Publish only after approval; the workflow itself leaves the release as a draft.

The workflow requires Apple Developer ID Application signing and notarization for macOS, in addition to the existing Tauri/Minisign update signatures. Windows uses the signed NSIS installer directly; Microsoft publisher signing remains unconfigured. macOS uses notarized `.app.tar.gz` updater bundles. These changes are not evidence that a publisher-signed release has already shipped: verify the actual draft artifacts before publishing.

### Apple publisher signing

The macOS bundle identifier is `com.firefungames.ratatoskr`. The Windows bundle/installer identity, configuration paths and background job labels remain stable. Desktop update history and restart intent deliberately retain their existing storage directory on both platforms; the desktop and CLI watchdog share its path. Existing skill sources, schedules and backups are not migrated or deleted. The first transition from the old ad-hoc signature to Developer ID may require a one-time macOS permission grant. Later releases must preserve the publisher, executable identifiers and signing requirements.

Create a GitHub environment named `release` and restrict deployment branches to `main` only. The workflow also refuses non-main dispatches. Only reviewed main-branch code may use the environment. Public fork pull requests and normal CI receive no Apple signing credentials. Pin every action in this secret-bearing workflow to its reviewed commit.

Environment secrets:

- `APPLE_CERTIFICATE`: base64 PKCS#12 containing the Developer ID Application certificate and private key.
- `APPLE_CERTIFICATE_PASSWORD`: the PKCS#12 password.
- `KEYCHAIN_PASSWORD`: an independent random password for disposable runner keychains.
- `APPLE_API_KEY_CONTENT`: the downloaded App Store Connect API private key used for notarization. Use the least role Apple permits for notarization, not an Apple Account password.

Environment variables (public identity, not signing authority):

- `APPLE_SIGNING_IDENTITY`: the actual certificate's `Developer ID Application: …` identity.
- `APPLE_TEAM_ID`: the issuing developer team.
- `APPLE_API_KEY` and `APPLE_API_ISSUER`: the notarization key ID and issuer ID.

Transfer credentials directly from the secret manager into GitHub over stdin. Never paste them into issues, PRs or workflow logs. Keep the recovery copy in the secret manager, outside the public tool and private skill-content repositories. A public certificate, Team ID or update public key cannot be used to sign software without the corresponding private key.

On a disposable macOS runner, the preparation step imports the certificate into a temporary keychain and writes the notarization key with owner-only permissions under `RUNNER_TEMP`. The temporary PKCS#12 is removed immediately after import. An `always()` cleanup removes the notarization key and keychain. Only explicit distributable files under `artifacts/` are uploaded; keychains, credentials and runner directories are never uploaded or cached. Credential values are scoped to the steps that need them.

The build requires the real signing identity and notarization configuration. Verification checks the Apple Developer ID certificate chain, configured team, app/helper identifiers, hardened runtime and secure timestamp. It also validates the app's stapled notarization ticket and Gatekeeper assessment. The standalone helper is separately submitted to Apple and must return `Accepted` before it is signed for the updater. Missing credentials, ad-hoc signing, a wrong team or rejected notarization stop release creation.

## Windows helper pair

## Windows tray migration

Windows now ships the tray-resident desktop app and the console CLI. The `ratatoskrw` Cargo target, feature, packaging, release asset, paired-update logic and checks have been removed. The six-platform manifest contains desktop and CLI assets for macOS Apple Silicon, macOS Intel and Windows x64.

The signed NSIS desktop updater used by v0.2.7 still consumes the same `windows-x86_64` installer asset. The updater passes `/UPDATE`, and [Tauri CLI 2.12.3's NSIS template](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.3/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi#L298) skips the old uninstaller in that mode. Therefore an existing `ratatoskrw.exe` is expected to remain after an in-app upgrade. The new bundle has no previous-version resource-ownership manifest with which to safely identify that file; we do not delete a sibling executable by filename alone. The leftover is inert after owned-task migration, and the app never launches it. A normal interactive upgrade that runs the old uninstaller can remove the old sidecar, but the in-app update path does not depend on that.

A v0.2.7 **standalone CLI** updater requires both old helper assets and therefore refuses the new manifest before replacing either binary. That old client cannot be changed by a new manifest. Upgrade that installation with the signed desktop installer, or download and verify the new standalone CLI manually. New standalone CLI versions self-update only the CLI. No compatibility executable or unsigned alias is published.

At desktop startup, legacy task deletion requires the existing ownership record and exact local XML bytes, plus the task label derived from the configuration path and the expected state-file path. Both historical UTF-8 and UTF-16LE+BOM definitions remain recognized. Disabled tasks are deleted through the same path as enabled tasks. Migration first enables hidden Open Ratatoskr at login through the per-user Run key, then removes the owned task and records the replacement logon startup in the ledger. A login-start failure preserves the old task. Migration preserves the configured interval and enables tray sync; edited/unowned tasks are left untouched and block tray sync, with an error in the UI and job-status ledger.

Run-key autostart retains the existing preference when there is no owned task to migrate. Owned-task migration enables it to preserve the previous logon trigger. Tray settings warn when login startup is disabled. An already enabled entry is refreshed to include `--background`. Hidden windows are created hidden, avoiding a startup flash. A one-shot restart-state file in app data carries current window visibility across NSIS updater relaunch and explicit restart; foreground reopening overrides stale background launch arguments. Notification identities use stable operation contexts and typed error categories, independently for sync, migration, updates and app health. Full diagnostics remain in the UI and ledger. A persisted app-wide cap allows at most one problem toast per hour across all categories; a pending distinct problem can notify once the hour expires, and recovery remains allowed. They remain in memory during ledger-write failures; pending saves retry without repeating delivered toasts. Migration checks run at startup and when background settings open, with an explicit retry for failures. Native Windows timers preserve the nightly update window; unsaved desktop edits and core operation locks defer installation.

CI registers only harmless, far-future `cmd.exe /c exit 0` fixtures. It tests owned enabled/disabled migration, old XML encodings, edited-byte refusal, preservation of an unowned task, idempotence, and refusal of `schedule install`. The same integration executable runs under a disposable non-administrator account, with task/account/profile/fixture cleanup. Native desktop builds and NSIS packaging remain required CI checks; no release is produced by this change.

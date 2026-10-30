# Releases and signed updates

## v0.2.19 release notes

- **Project skill work no longer alerts on every save.** Background sync used to fail and alert as soon as a registered project had uncommitted or unpushed skill work. The alert included file counts, so each additional edited file sent a new notification. Sync now succeeds until work has been pending for over an hour, tracked separately for each skill, for other project skill source files and for unpushed commits. The alert then names those items without counts, so continued editing does not repeat it. Project inspection errors still alert immediately. After upgrading, expect one recovery notification if the previous alert was still current.
- **Former target locations are now explicit decisions.** Removing a harness from a machine, switching between staging and live, or changing a target path no longer fails on ownership left behind. Plan lists each former location as a pending decision, and Apply, reviewed Apply and background sync wait until every decision is resolved. **Remove** archives the previously owned files beside their former target with a recovery receipt. **Forget** stops managing them without touching any file. Both are available in **Review changes → Former target locations** and through `ratatoskr stale-target list|preview|resolve`. See [former target locations](usage.md#former-target-locations).
- `apply` reports the files it added, updated and removed, in text and in JSON `applied`, instead of only saying generated files are up to date.
- `validate` warns about selector values that can never match, such as a misspelled harness, OS or `[target]` entry. The warnings do not change the exit code.
- Finder `.DS_Store` files and Python `.pyc` files beside skill directories and inside `variants/` are ignored instead of blocking every plan. Other stray files, symlinks and Windows junctions are refused by name.
- Conflict copy-back and source-edit previews label their diff sides correctly. Previously the diff read backwards.
- Invalid identifiers now state the accepted form.
- Fix the desktop asking about unsaved changes right after a new skill was created.

## v0.2.18 release notes

- **Desktop edits to the central content checkout are now saved, committed and pushed to the deploy branch's upstream automatically.** This applies to global instructions, skills, configuration, agent targets, conflict copy-back and source restoration. Only the changed paths are committed, with your normal Git identity, signing and hooks. A failed commit keeps the saved file. A failed push keeps the local commit and reports **saved and committed locally but NOT pushed**. A persistent warning shows uncommitted and unpushed content, and **Push now** retries. Project edits and all CLI `skill` commands remain write-only.
- `ratatoskr status` reports central content Git state and exits 1 for uncommitted files, unpushed commits or unknown upstream state. Sync refuses unpushed central commits as an alerted problem instead of deploying unpublished content.
- Fix "Project is not registered" on Windows for `skill` commands that take `--project`, such as `preview`, `read`, `create` and `write`, and for `project remove`. Registration stores verbatim `\\?\` paths, and these commands now resolve a project the same way. The plain or verbatim form, a trailing separator, either drive-letter case and forward slashes all work.
- `project remove` now fails for a root that is not registered, instead of reporting success without changing anything. On Windows, a root spelled differently from the registry was previously left registered with its output ownership intact. A registered root that no longer exists can still be removed by its registered path, as shown by `status`.

## v0.2.17 release notes

- Search skills and global instructions with Ctrl/⌘F or the toolbar's Find button, including next/previous, match case, regular expressions and replace.
- Add `ratatoskr update --desktop` so agents and scripts can update the desktop app without its UI. The bundled CLI queues the request, and the running app installs the signed update with the nightly path's checks, then restarts hidden. `--status` reports the result. The app is started hidden if it isn't running.
- Stop unattended launches from taking focus. Login launches, CLI requests and the restarts after nightly or requested installs no longer activate the app on macOS, and those restarts keep the window hidden. Reopen it from the Dock or tray.

## v0.2.16 release notes

- Add a **Global instructions** screen for the shared `global/AGENTS.md`, with the skill editor's conditions, values and live per-agent × machine preview, including each agent's destination file. Saving checks every machine profile and refuses new problems, blank text or a stale source. The generic source save no longer accepts this file because it checked only the current machine.
- Restore **Archive** in the skill editor. It moves the skill into `.ratatoskr-archive` without deleting anything and shows the archive path; installed copies are removed on the next Apply. Archiving from the CLI or desktop is now refused while an apply is running.
- Remove nine unused desktop commands left over from the redesign. Newer commands or the workspace snapshot already cover them.
- Require Apple Developer ID signing and notarization for official macOS release builds, including verification of the desktop app and bundled helper. Release automation stops if publisher credentials or notarization acceptance are missing.
- Use `com.firefungames.ratatoskr` as the macOS bundle identifier while preserving existing configuration, update history, restart intent and background-job continuity. Windows identity and the update-signature trust root remain unchanged.
- Build candidates in CI, then sign and notarize the Mac artifacts on a local publisher Mac. Apple private keys stay in the local encrypted secret manager. Real signed-artifact and old-install update acceptance are still required before publication.

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

The official key pair was generated with the Tauri signer. Its private key and password are stored in gopass and the main-only GitHub Actions `release` environment:

- `ratatoskr/release/tauri-signing-key` → `TAURI_SIGNING_PRIVATE_KEY`
- `ratatoskr/release/tauri-signing-key-password` → `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

Load each value directly from gopass into `gh secret set --env release` over stdin. Never print secrets or write them to local files. Repository-level copies must not remain: when migrating an existing setup, verify a successful Windows candidate's update signatures using the environment copies before removing the old scope. The release build reads the committed public key from `tauri.conf.json`; no repository public-key variable is required. A release build verifies that the private and public keys agree.

1. Set matching versions in workspace `Cargo.toml`, `desktop/package.json`, and `desktop/src-tauri/tauri.conf.json`; refresh the lockfiles. Version changes require the normal PR, green CI, and approval from an agent on another harness. The author merges after approval unless Oana grants a specific bypass.
2. Dispatch **Build release candidates** from `main`. It builds Apple Silicon and Intel Mac app candidates without Apple credentials, plus the Windows x64 updater and helper with their existing update signatures. The bundled helper is prepared from the same source version. The workflow uploads candidates and cannot create or publish a release.
3. On the local publisher Mac, install the locked desktop dependencies and check out the reviewed commit at `origin/main`. Run `python3 scripts/macos-signing.py release --help` for the required run ID, public certificate, team and gopass-entry options. The command refuses a dirty checkout or a candidate run that is not a successful main-branch dispatch for that exact commit. It signs and notarizes both Mac architectures, verifies every update signature against the existing trust root, and creates `latest.json` with all six platforms. `--draft` additionally creates a GitHub draft using only the verified distributables.
4. Test the draft artifacts on actual Macs and Windows, including an older installed version updating to the candidate. Publish only after owner approval. The local command cannot publish a release.

Final macOS distributables require Apple Developer ID Application signing and notarization, in addition to the existing Tauri/Minisign update signatures. Windows uses the signed NSIS installer directly; Microsoft publisher signing remains unconfigured. macOS uses notarized `.app.tar.gz` updater bundles. Candidate builds are not evidence that a publisher-signed release has shipped.

### Apple publisher signing

The macOS bundle identifier is `com.firefungames.ratatoskr`. The Windows bundle/installer identity, configuration paths and background job labels remain stable. Desktop update history and restart intent deliberately retain their existing storage directory on both platforms; the desktop and CLI watchdog share its path. Existing skill sources, schedules and backups are not migrated or deleted. The first transition from the old ad-hoc signature to Developer ID may require a one-time macOS permission grant. Later releases must preserve the publisher, executable identifiers and signing requirements.

Apple signing authority stays off GitHub entirely. Keep the certificate's private key and the notarization API key only in the local encrypted secret manager. Do not configure Apple secrets in GitHub Actions. The main-only `release` environment holds only the existing Tauri update-signing credentials needed by the Windows job. Pin release actions to reviewed commits.

The local publisher requires macOS 27 or later (for the encrypted `diskutil image` API), full Xcode, Python 3.9 or later, OpenSSL, gopass, Node, Cargo and the GitHub CLI. It reads the certificate key from a gopass PEM entry. The notary entry is one encrypted JSON document containing `private_key` (the downloaded PKCS#8 PEM), `key_id` and `issuer_id`. Entry names, the public DER certificate path and team are supplied as command options; private machine profiles and actual credentials do not belong in this repository.

The certificate is matched to its private key in memory, then imported into a temporary encrypted Keychain. Its encrypted PKCS#12 transport uses Keychain-compatible algorithms and a fresh random password; it is removed immediately after import. Password-bearing Keychain commands travel over stdin rather than process arguments. Both Mac candidates are signed before the temporary Keychain is deleted; it does not remain open during notarization waits. `check-identity` verifies this path on a disposable compiled probe without creating a release.

Apple's notarization tool requires a regular key file and rejects pipes. The local command therefore exposes the key only inside a temporary AES-256 encrypted disk image, with an owner-only mount and key file. The image password stays in memory. The key file is removed before detaching the image; a failed detach is retried with force for this owned temporary volume only. Cleanup failures retain the original error. SIGTERM and SIGHUP trigger cleanup as well. SIGKILL and power loss cannot run cleanup and may leave a temporary Keychain search-list entry or credential volume; inspect and remove only the interrupted run's containers before retrying. No unencrypted `.p8` is written to the host filesystem. Unlocked local credential containers remain accessible to the signing process and other processes running as that user, so signing requires a trusted local Mac.

Verification checks the Apple Developer ID chain, team, app/helper identifiers, hardened runtime and secure timestamp. Both the app and standalone helper must be accepted by Apple. Rejections preserve their nonsecret notary diagnostics. Candidates reject links, special files and AppleDouble metadata; CI strips macOS metadata from their ZIPs. The app is stapled before Python creates the updater tarball, without synthesizing `._*` entries. The tarball's member paths and types are validated before Python extraction; the extracted copy must pass publisher-signature, stapled-ticket and Gatekeeper checks. Every update signature is checked against the existing Rust trust root and its authenticated release version, including downloaded Windows artifacts. Existing tags or draft releases stop the operation before credential use and are checked again before creating a draft. No Keychain, image, secret-manager entry, candidate ZIP or diagnostic log is uploaded to the draft.

## Windows tray migration

Windows now ships the tray-resident desktop app and the console CLI. The `ratatoskrw` Cargo target, feature, packaging, release asset, paired-update logic and checks have been removed. The six-platform manifest contains desktop and CLI assets for macOS Apple Silicon, macOS Intel and Windows x64.

The signed NSIS desktop updater used by v0.2.7 still consumes the same `windows-x86_64` installer asset. The updater passes `/UPDATE`, and [Tauri CLI 2.12.3's NSIS template](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.3/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi#L298) skips the old uninstaller in that mode. Therefore an existing `ratatoskrw.exe` is expected to remain after an in-app upgrade. The new bundle has no previous-version resource-ownership manifest with which to safely identify that file; we do not delete a sibling executable by filename alone. The leftover is inert after owned-task migration, and the app never launches it. A normal interactive upgrade that runs the old uninstaller can remove the old sidecar, but the in-app update path does not depend on that.

A v0.2.7 **standalone CLI** updater requires both old helper assets and therefore refuses the new manifest before replacing either binary. That old client cannot be changed by a new manifest. Upgrade that installation with the signed desktop installer, or download and verify the new standalone CLI manually. New standalone CLI versions self-update only the CLI. No compatibility executable or unsigned alias is published.

At desktop startup, legacy task deletion requires the existing ownership record and exact local XML bytes, plus the task label derived from the configuration path and the expected state-file path. Both historical UTF-8 and UTF-16LE+BOM definitions remain recognized. Disabled tasks are deleted through the same path as enabled tasks. Migration first enables hidden Open Ratatoskr at login through the per-user Run key, then removes the owned task and records the replacement logon startup in the ledger. A login-start failure preserves the old task. Migration preserves the configured interval and enables tray sync; edited/unowned tasks are left untouched and block tray sync, with an error in the UI and job-status ledger.

Run-key autostart retains the existing preference when there is no owned task to migrate. Owned-task migration enables it to preserve the previous logon trigger. Tray settings warn when login startup is disabled. An already enabled entry is refreshed to include `--background`. Hidden windows are created hidden, avoiding a startup flash. A one-shot restart-state file in app data carries current window visibility across NSIS updater relaunch and explicit restart; foreground reopening overrides stale background launch arguments. Notification identities use stable operation contexts and typed error categories, independently for sync, migration, updates and app health. Full diagnostics remain in the UI and ledger. A persisted app-wide cap allows at most one problem toast per hour across all categories; a pending distinct problem can notify once the hour expires, and recovery remains allowed. They remain in memory during ledger-write failures; pending saves retry without repeating delivered toasts. Migration checks run at startup and when background settings open, with an explicit retry for failures. Native Windows timers preserve the nightly update window; unsaved desktop edits and core operation locks defer installation.

CI registers only harmless, far-future `cmd.exe /c exit 0` fixtures. It tests owned enabled/disabled migration, old XML encodings, edited-byte refusal, preservation of an unowned task, idempotence, and refusal of `schedule install`. The same integration executable runs under a disposable non-administrator account, with task/account/profile/fixture cleanup. Native desktop builds and NSIS packaging remain required CI checks; no release is produced by this change.

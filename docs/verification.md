# Verification

Initial draft prepared on macOS Apple Silicon on 2026-10-16. The owner explicitly authorized a direct push to main for this draft and plans a later Claude review. Normal subsequent changes follow AGENTS.md.

## Local results

- 26 automated core/CLI workflow tests passed, covering rendering, conditions, variants, malformed source, ownership boundaries, concurrent edits, backup/restore, project rendering and Git status, scheduling definitions, setup, and signed payload tampering.
- Rust formatting and strict Clippy checks passed for core, CLI, and native desktop.
- TypeScript, Prettier, and Vite production build passed.
- Actual CLI exercised in an isolated temporary content repository: initialization, skill creation, per-harness output, enable/disable, apply, idempotence, and pull/sync through a local bare Git upstream.
- The launchd XML passed Apple's `plutil -lint` check. An isolated launchd job was installed, ran sync with exit code 0, and was removed along with its ownership record.
- Native Mac executable built. Tauri packaged the Mac `.app`; its Info.plist, squirrel icon, and runnable independent helper were checked.
- Browser preview inspected for content, loaded icon, absence of horizontal overflow at desktop size, and disabled filesystem controls outside Tauri.
- A real Mac updater archive and signature were built successfully using a disposable matching key pair. Release artifact collection was exercised on that archive.
- Update fixture generated using Tauri's signer. The public fixture and signature are committed; its temporary private key is not part of the repository.

## Remaining verification and setup

- CI runs core tests on Linux, macOS and Windows, and desktop strict checks and native binary builds on macOS and Windows. Consult the current GitHub Actions result for remote outcomes.
- Native interaction automation was unavailable on the development host. Building/launching the Mac executable and inspecting the browser frontend do not prove every native UI interaction.
- A signed production release and an actual update from an older installed version have not been exercised. The official production signing key and both GitHub Actions secrets are configured; full release installation still requires acceptance testing.
- Windows Task Scheduler registration, installer behavior, and a real Windows desktop session still require live acceptance. CI compilation does not establish those runtime results.
- Apple notarization and Microsoft publisher signing remain unconfigured. No published release binaries exist at this stage.

The existing private skill deployment, live agent folders, and scheduled agentfiles job were not switched to Ratatoskr.

## Official updates and deployment branch changes

Local validation for this change uses Rust formatting, strict Clippy for core/CLI/native desktop, core workflow tests, desktop UI policy tests, TypeScript, Prettier, and the Vite production build. CI runs core checks on Linux/macOS/Windows and desktop checks/builds on macOS/Windows; the PR check results are authoritative for remote runs.

The official key pair was read back from gopass to sign the existing public test payload with Tauri's signer. The committed public signature verifies against the compiled official key in the core suite; private values never entered files. Both release secret names were confirmed through GitHub metadata.

New coverage exercises:

- Official endpoint/key defaults, explicit field-by-field override precedence, rejected invalid overrides, matching native updater configuration, and visible UI override indication.
- Injected local clocks before/at/after the chosen time, the bounded one-hour desktop window, windows crossing midnight, disabled updates, and daily accounting across local offset changes.
- A persisted helper attempt, including network failure, no second attempt that day, next-day retry, and corrupt-state refusal.
- Frontend check/defer/install flow: no install/restart without an update, unsaved editors and busy operations, edits started during the network check, subsequent idle retry, and propagated check/install failures.
- Cross-process apply/sync/schedule lock contention defers desktop installation; acquired update guards prevent those operations starting during installation.
- Wrong deployment branch, wrong upstream, missing upstream, and detached HEAD refuse before fetch/apply and record alerts. Returning to the configured branch/upstream permits sync. The local bare remote uses a relative path, avoiding Windows verbatim paths being parsed as SSH hostnames.
- A failed helper update check still applies synced content and raises its own deduplicated alert. Skipped checks preserve that alert, simultaneous sync failures alert separately, and failed update-alert delivery still permits sync.
- Persisted desktop launch decisions cover missed nightly windows, no repeat check after a completed attempt, a deferred available update, changed settings, midnight-spanning windows, disabled updates, and corrupt-state errors. Frontend startup tests keep interaction blocked until the update decision/install completes and propagate failures after restoring workspace access.
- Update guards explicitly unlock even when another duplicated/inherited handle remains open, preventing concurrent process launches from delaying lock release.

No public release was cut. An actual installed desktop downloading, installing, and relaunching a newer published version, Windows installer behavior, and unattended midnight operation on a real machine remain acceptance steps. Automated clock/policy tests and native builds establish the implemented decisions, not those platform acceptance results. The existing private skill deployment and live agent schedules were not switched to Ratatoskr.


## Desktop workflows in 0.2.0

Local coverage adds reviewed conflict copy-back, reinstall, adoption, manual template-source editing, stale-preview rejection, removed files, executable permissions, unmanaged directories and backup/restore round trips. Draft previews use the deployment renderer and preserve the selected machine's template context even when its live OS differs from the host. Included presets and separate instruction roots are validated. Helper discovery checks bundled siblings before standalone PATH installations and reports absence.

Sync records last start/completion/result and a bounded history independently of configured notifications. Current errors come from the deduplicated alert ledgers; failed deliveries remain retryable. Desktop tests exercise debounce, stale asynchronous responses, cancellation and propagated validation failures, alongside the existing update policy suite.

Open at login is off until explicitly enabled through the Tauri autostart plugin. A real reboot/login launch and the published 0.1.0-to-0.2.0 auto-update remain platform acceptance steps. Windows Task Scheduler and Windows desktop interaction still require a live Windows session. Preset documentation is verified; discovery inside each individual agent app has not been exercised. See [agent paths and explicit omissions](agent-presets.md).

PR #2 also exposes parser analysis and focused command endpoints for the separate UI redesign; see [desktop-api.md](desktop-api.md). All ranges are UTF-16 offsets. A regression test proves operation locks explicitly release even while another process retains a duplicated file descriptor. CLI acceptance exercised the new actions in an isolated temporary repository. The native desktop builds; its new command flows were not verified through native UI automation.

Final local PR #2 checks: 59 Rust tests and 11 desktop tests passed, Rust formatting/strict Clippy passed for default targets and native desktop, locked helper/desktop builds passed, and npm ci/format/check/build passed. Real CLI acceptance was repeated against the final core. Native UI interaction, login after reboot, individual agent discovery and production auto-update remain unverified platform acceptance steps.

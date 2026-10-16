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
- Wrong deployment branch, wrong upstream, missing upstream, and detached HEAD refuse before fetch/apply and record alerts. Returning to the configured branch/upstream permits sync.

No public release was cut. An actual installed desktop downloading, installing, and relaunching a newer published version, Windows installer behavior, and unattended midnight operation on a real machine remain acceptance steps. Automated clock/policy tests and native builds establish the implemented decisions, not those platform acceptance results. The existing private skill deployment and live agent schedules were not switched to Ratatoskr.

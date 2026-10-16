# First-draft verification

Initial draft prepared on macOS Apple Silicon on 2026-10-16. The owner explicitly authorized a direct push to main for this draft and plans a later Claude review. Normal subsequent changes follow AGENTS.md.

## Local results

- 25 automated core/CLI workflow tests passed, covering rendering, conditions, variants, malformed source, ownership boundaries, concurrent edits, backup/restore, project rendering and Git status, scheduling definitions, setup, and signed payload tampering.
- Rust formatting and strict Clippy checks passed for core, CLI, and native desktop.
- TypeScript, Prettier, and Vite production build passed.
- Actual CLI exercised in an isolated temporary content repository: initialization, skill creation, per-harness output, enable/disable, apply, idempotence, and pull/sync through a local bare Git upstream.
- The launchd XML passed Apple's `plutil -lint` check. An isolated launchd job was installed, ran sync with exit code 0, and was removed along with its ownership record.
- Native Mac executable built. Tauri packaged the Mac `.app`; its Info.plist, squirrel icon, and runnable independent helper were checked.
- Browser preview inspected for content, loaded icon, absence of horizontal overflow at desktop size, and disabled filesystem controls outside Tauri.
- A real Mac updater archive and signature were built successfully using a disposable matching key pair. Release artifact collection was exercised on that archive.
- Update fixture generated using Tauri's signer. The public fixture and signature are committed; its temporary private key is not part of the repository.

## Remaining verification and setup

- CI runs core tests on Linux, macOS and Windows, and desktop strict checks/build preparation on macOS and Windows. Consult the current GitHub Actions result for remote outcomes.
- Native interaction automation was unavailable on the development host. Building/launching the Mac executable and inspecting the browser frontend do not prove every native UI interaction.
- A signed production release and an actual update from an older installed version have not been exercised. Production signing-key setup is required before the draft-release workflow can run.
- Windows Task Scheduler registration, installer behavior, and a real Windows desktop session still require live acceptance. CI compilation does not establish those runtime results.
- Apple notarization and Microsoft publisher signing remain unconfigured. No published release binaries exist at this stage.

The existing private skill deployment, live agent folders, and scheduled agentfiles job were not switched to Ratatoskr.

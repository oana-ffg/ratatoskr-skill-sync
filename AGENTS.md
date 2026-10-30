# Ratatoskr Skill Sync

This public MIT repository contains the tool, not Oana's private skills or machine profiles.
The CLI and optional desktop UI share the Rust core; syncing must work with the main window closed. Windows requires the tray app to remain running; macOS uses launchd.
Preserve intentional differences between machines, operating systems, and harnesses.
Run `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
For desktop changes also run `npm ci`, `npm run check`, and `npm run build` from `desktop/`.
Never run the native desktop journeys (`RATATOSKR_NATIVE_E2E=1`) locally; they run in CI only, because the test app shares its single-instance record and app data with an installed Ratatoskr.
Changes require a PR with green CI and approval by an agent using a different harness. The author merges after approval.
Nothing merges without that review unless Oana explicitly grants a bypass for that specific change.

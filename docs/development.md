# Development

## Build from source

Install stable Rust (1.90 or newer), Git, Node.js 22.18 or newer, and [Tauri's platform prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
cargo build --locked
cargo run -- --help
cd desktop
npm ci
npm run tauri dev
```

Windows uses the desktop app for tray-resident sync. `cargo build --locked` also builds the standalone console CLI.

To package a native app, run `npm run tauri build` inside `desktop`. The build prepares and bundles the independent helper. The browser-only `npm run dev` preview has no filesystem access. The screenshot tests install mocked IPC using disposable public fixtures, never your real configuration.

## Development checks

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cd desktop
npm ci
npm run format:check
npm test
npm run check
npm run build
npx playwright install chromium webkit
npm run test:ui
npm run test:screenshots
```

`test:ui` drives the real Svelte/CodeMirror editor in Chromium and WebKit, with
the Rust core validating and saving public fixtures in disposable directories.
CI runs it as `editor-ui` and retains screenshots and traces on failure. Cover
user-visible failure and recovery paths when changing editor behavior, including
pending preview, invalid drafts, source conflicts, refresh and target selection.

`cargo test --workspace` and non-bundling Cargo checks do not need a prebuilt helper. When it is absent, the build excludes the sidecar from that non-bundling build and reports how to prepare it. macOS background jobs require the real CLI (`npm run prepare:helper -- --debug` from `desktop/`). Windows tray sync calls the core directly. Native packaging still requires and bundles the real CLI; the Tauri CLI prepares it automatically. Rust's platform prerequisites still apply to workspace tests.

For changes to Windows-only desktop code, also compile the Windows cfg paths before pushing. Install the target once, then run these commands from the repository root and from `desktop/src-tauri/`:

```sh
rustup target add x86_64-pc-windows-msvc
cargo check --locked --workspace --all-targets --target x86_64-pc-windows-msvc
cargo clippy --locked --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
```

These commands do not link the app, but dependencies can still compile native code. Cross-checking from macOS requires Windows SDK headers and compatible C/resource tools; installing the Rust target alone is insufficient (`ring` can fail with `fatal error: 'assert.h' file not found`). If native prerequisites block the check, record the exact build-script failure in the PR and require the Windows CI jobs to pass before merging. A macOS build does not compile `#[cfg(windows)]` paths.


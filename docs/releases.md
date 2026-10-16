# Releases and signed updates

The initial draft has no published binaries or configured production signing key. Builds can be tested locally without publisher signing. The signed-release workflow requires its own update key before it can run.

## Prepare a release

1. From `desktop`, run `npm run tauri -- signer generate -w` with a new secure private-key path. Keep the private key and password outside this repository. Retain an offline backup.
2. Add the generated private-key file contents as the GitHub Actions secret `TAURI_SIGNING_PRIVATE_KEY`. Add its password as `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Do not paste either into issues, source, or chats.
3. Set matching versions in workspace `Cargo.toml`, `desktop/package.json`, and `desktop/src-tauri/tauri.conf.json`; refresh both lockfiles. Version bumps must go through the normal review workflow.
4. Dispatch **Build signed draft release**. It builds Apple Silicon, Intel Mac, and Windows x64 desktop bundles, plus separate raw helper executables. The bundled helper is prepared from the same source version.
5. Download and test the artifacts on actual Macs and Windows before publishing the draft release. Inspect `latest.json`, including all six platforms. Publish the draft when ready. The workflow does not publish it automatically.

The workflow signs update artifacts with Tauri/Minisign. The Windows updater uses the signed NSIS installer directly; the legacy ZIP-updater feature is disabled. macOS uses signed `.app.tar.gz` updater bundles. This avoids an unused ZIP dependency whose old versions were yanked upstream during initial development.

The workflow does not provide Apple Developer ID signing/notarization or Microsoft Artifact Signing. Those are separate optional CI integrations, and first-run platform trust prompts may remain. A Minisign signature authenticates updates; it does not establish an OS publisher identity.

## Enable updates

In the app's local TOML configuration, add an `[updates]` table. Set `endpoint` to this repository's HTTPS latest-release asset endpoint: `https://github.com/oana-ffg/ratatoskr-skill-sync/releases/latest/download/latest.json`. Set `public_key` to the generated `.pub` file contents. Set `automatic = true` only if automatic installation is wanted; the default is false.

Use the public key from the trusted release setup, not a key downloaded from an untrusted manifest. Public keys may be Tauri's base64-encoded public-key file or a raw Minisign public-key line.

The CLI's `update` command checks; `update --install` installs. Automatic standalone-helper updates run after successful `sync`. Desktop updates check on startup when automatic is enabled. A helper bundled in a Mac app or beside the Windows desktop executable is updated through the desktop bundle, not independently. Restart the app after a desktop update.

A standalone helper update checks HTTPS, size limits, a valid Minisign signature, a newer semantic version, and agreement between the signed binary's `--version` output and the manifest. Tauri verifies desktop signatures and requires the signed app version to match the manifest. Treat signing-key rotation as an explicit trust migration; editing an unsigned manifest cannot change the locally trusted public key.

The packaged updater configuration intentionally has no static trusted key (`pubkey` is empty). Every update operation first validates the local update settings and supplies that trusted key at runtime. This supports separate content deployments and prevents a development/test key from becoming a production trust root.

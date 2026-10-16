import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const [target, bundle] = process.argv.slice(2);
if (!target || !['app', 'nsis'].includes(bundle)) throw new Error('Expected Rust target triple and app/nsis bundle');
const publicKey = JSON.parse(readFileSync('desktop/src-tauri/tauri.conf.json')).plugins.updater.pubkey;
if (!publicKey || !process.env.TAURI_SIGNING_PRIVATE_KEY) throw new Error('Configure TAURI_SIGNING_PRIVATE_KEY before building a signed release');
const version = JSON.parse(readFileSync('desktop/package.json')).version;
const tauriVersion = JSON.parse(readFileSync('desktop/src-tauri/tauri.conf.json')).version;
const cargoVersion = readFileSync('Cargo.toml', 'utf8').match(/^version = "([^"]+)"$/m)?.[1];
if (version !== tauriVersion || version !== cargoVersion) throw new Error('Workspace, desktop package, and Tauri versions must match');
const configuration = JSON.stringify({ bundle: { createUpdaterArtifacts: true }, plugins: { updater: { pubkey: publicKey } } });
const result = spawnSync(process.execPath, [resolve('desktop/node_modules/@tauri-apps/cli/tauri.js'), 'build', '--target', target, '--bundles', bundle, '--config', configuration], { cwd: 'desktop', stdio: 'inherit' });
if (result.error) throw result.error;
process.exit(result.status ?? 1);

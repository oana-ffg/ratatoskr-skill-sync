import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const [target, bundle] = process.argv.slice(2);
const bundles = { 'aarch64-apple-darwin': 'app', 'x86_64-apple-darwin': 'app', 'x86_64-pc-windows-msvc': 'nsis' };
if (!Object.hasOwn(bundles, target) || bundles[target] !== bundle) throw new Error('Expected a supported Rust target and its app/nsis bundle');
const publicKey = JSON.parse(readFileSync('desktop/src-tauri/tauri.conf.json')).plugins.updater.pubkey;
if (!publicKey) throw new Error('The update-signature trust root is required');
const version = JSON.parse(readFileSync('desktop/package.json')).version;
const tauriVersion = JSON.parse(readFileSync('desktop/src-tauri/tauri.conf.json')).version;
const cargoVersion = readFileSync('Cargo.toml', 'utf8').match(/^version = "([^"]+)"$/m)?.[1];
if (version !== tauriVersion || version !== cargoVersion) throw new Error('Workspace, desktop package, and Tauri versions must match');
const macOS = target.endsWith('-apple-darwin');
if (macOS) {
  if (bundle !== 'app') throw new Error('macOS CI produces app candidates for local publisher signing');
} else if (!process.env.TAURI_SIGNING_PRIVATE_KEY) {
  throw new Error('Configure TAURI_SIGNING_PRIVATE_KEY before building a signed Windows updater');
}
const configuration = JSON.stringify({ bundle: {
  createUpdaterArtifacts: !macOS,
  ...(macOS ? { macOS: { signingIdentity: '-' } } : {}),
}, plugins: { updater: { pubkey: publicKey } } });
const result = spawnSync(process.execPath, [resolve('desktop/node_modules/@tauri-apps/cli/tauri.js'), 'build', '--target', target, '--bundles', bundle, '--config', configuration, '--', '--locked'], { cwd: 'desktop', stdio: 'inherit' });
if (result.error) throw result.error;
process.exit(result.status ?? 1);

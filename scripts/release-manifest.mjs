import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, join } from 'node:path';

const [directory, version, repository] = process.argv.slice(2);
if (!directory || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version || '') || !/^[\w.-]+\/[\w.-]+$/.test(repository || '')) {
  throw new Error('Usage: node scripts/release-manifest.mjs ARTIFACT_DIRECTORY VERSION OWNER/REPO');
}
function files(root) {
  return readdirSync(root, { withFileTypes: true }).flatMap(entry => entry.isDirectory() ? files(join(root, entry.name)) : [join(root, entry.name)]);
}
const all = files(directory);
const platforms = {};
for (const signature of all.filter(path => path.endsWith('.sig'))) {
  const asset = signature.slice(0, -4);
  const name = basename(asset);
  let platform;
  if (name.startsWith('ratatoskr-darwin-aarch64')) platform = 'darwin-aarch64-cli';
  else if (name.startsWith('ratatoskr-darwin-x86_64')) platform = 'darwin-x86_64-cli';
  else if (name.startsWith('ratatoskr-windows-x86_64')) platform = 'windows-x86_64-cli';
  else if (name === 'ratatoskrw-windows-x86_64.exe') platform = 'windows-x86_64-windowless';
  else if (name.endsWith('.app.tar.gz')) platform = name.includes('darwin-aarch64') ? 'darwin-aarch64' : name.includes('darwin-x86_64') ? 'darwin-x86_64' : undefined;
  else if (name.endsWith('-setup.exe')) platform = 'windows-x86_64';
  if (!platform) throw new Error(`Unrecognized signed artifact: ${name}`);
  if (platforms[platform]) throw new Error(`Duplicate platform ${platform}`);
  platforms[platform] = { url: `https://github.com/${repository}/releases/download/v${version}/${encodeURIComponent(name)}`, signature: readFileSync(signature, 'utf8').trim() };
}
const required = ['darwin-aarch64', 'darwin-x86_64', 'windows-x86_64', 'darwin-aarch64-cli', 'darwin-x86_64-cli', 'windows-x86_64-cli', 'windows-x86_64-windowless'];
for (const platform of required) if (!platforms[platform]) throw new Error(`Missing signed artifact for ${platform}`);
writeFileSync(join(directory, 'latest.json'), JSON.stringify({ version, notes: `Ratatoskr Skill Sync ${version}`, pub_date: new Date().toISOString(), platforms }, null, 2) + '\n');

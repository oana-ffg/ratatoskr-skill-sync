import { readdirSync, mkdirSync, copyFileSync } from 'node:fs';
import { join } from 'node:path';
const [source, destination, platform, version] = process.argv.slice(2);
if (!source || !destination || platform !== 'windows-x86_64' || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version || '')) throw new Error('Expected bundle directory, artifact directory, windows-x86_64, and semantic version');
function files(root) {
  return readdirSync(root, { withFileTypes: true }).flatMap(entry => entry.isDirectory() ? files(join(root, entry.name)) : [join(root, entry.name)]);
}
const matches = files(source).filter(path => path.endsWith('-setup.exe'));
if (matches.length !== 1) throw new Error(`Expected one signed desktop bundle, found ${matches.length}`);
const output = join(destination, `Ratatoskr-Skill-Sync-${version}-${platform}-setup.exe`);
mkdirSync(destination, { recursive: true });
copyFileSync(matches[0], output);
copyFileSync(`${matches[0]}.sig`, `${output}.sig`);

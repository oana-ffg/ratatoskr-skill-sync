import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const script = fileURLToPath(new URL('../release-manifest.mjs', import.meta.url));
const assets = [
  'ratatoskr-darwin-aarch64', 'ratatoskr-darwin-x86_64', 'ratatoskr-windows-x86_64.exe',
  'ratatoskrw-windows-x86_64.exe', 'Ratatoskr-darwin-aarch64.app.tar.gz',
  'Ratatoskr-darwin-x86_64.app.tar.gz', 'Ratatoskr-setup.exe',
];
test('release manifest requires and independently signs the Windows helper pair', () => {
  const directory = mkdtempSync(join(tmpdir(), 'ratatoskr-manifest-'));
  try {
    for (const name of assets) {
      writeFileSync(join(directory, name), 'test artifact');
      writeFileSync(join(directory, `${name}.sig`), `test signature for ${name}\n`);
    }
    const run = () => spawnSync(process.execPath, [script, directory, '0.2.7', 'oana-ffg/ratatoskr-skill-sync'], { encoding: 'utf8' });
    const result = run();
    assert.equal(result.status, 0, result.stderr);
    const manifest = JSON.parse(readFileSync(join(directory, 'latest.json')));
    assert.equal(Object.keys(manifest.platforms).length, 7);
    assert.match(manifest.platforms['windows-x86_64-cli'].url, /\/ratatoskr-windows-x86_64\.exe$/);
    assert.match(manifest.platforms['windows-x86_64-windowless'].url, /\/ratatoskrw-windows-x86_64\.exe$/);
    assert.equal(manifest.platforms['windows-x86_64-windowless'].signature, 'test signature for ratatoskrw-windows-x86_64.exe');
    rmSync(join(directory, 'ratatoskrw-windows-x86_64.exe.sig'));
    const missing = run();
    assert.notEqual(missing.status, 0);
    assert.match(missing.stderr, /Missing signed artifact for windows-x86_64-windowless/);
  } finally {
    rmSync(directory, { recursive: true });
  }
});

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const python = process.platform === 'win32' ? 'python' : 'python3';
const script = 'scripts/macos-signing.py';

test('rejected and in-progress Apple submissions cannot pass release validation', () => {
  const root = mkdtempSync(join(tmpdir(), 'ratatoskr-notary-test-'));
  try {
    const file = join(root, 'submission.json');
    for (const status of ['Invalid', 'In Progress', 'Rejected', 'Accepted']) {
      writeFileSync(file, JSON.stringify({ status }));
      const result = spawnSync(python, [script, 'accepted', file]);
      assert.equal(result.status, status === 'Accepted' ? 0 : 1);
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('a valid ad-hoc signature is never accepted as a publisher signature', { skip: process.platform !== 'darwin' }, () => {
  const root = mkdtempSync(join(tmpdir(), 'ratatoskr-publisher-test-'));
  try {
    const source = join(root, 'main.c');
    const binary = join(root, 'ratatoskr');
    writeFileSync(source, 'int main(void) { return 0; }\n');
    assert.equal(spawnSync('cc', [source, '-o', binary]).status, 0);
    assert.equal(spawnSync('codesign', ['--force', '--sign', '-', '--identifier', 'ratatoskr', binary]).status, 0);
    assert.equal(spawnSync('codesign', ['--verify', '--strict', binary]).status, 0);
    const result = spawnSync(python, [script, 'verify', binary, 'ratatoskr'], {
      env: { ...process.env, APPLE_TEAM_ID: 'ABCDE12345' },
    });
    assert.equal(result.status, 1);
    assert.match(result.stderr.toString(), /signing command failed/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

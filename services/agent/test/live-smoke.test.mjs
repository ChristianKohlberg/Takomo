// Opt-in: runs the installed, authenticated Codex against a disposable committed
// fixture and requires real repository evidence. Never part of CI.
//   TAKOMO_AGENT_LIVE_SMOKE=1 node --test services/agent/test/live-smoke.test.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir, homedir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { Codex } from '../codex.mjs';

const enabled = process.env.TAKOMO_AGENT_LIVE_SMOKE === '1';
test('real Codex research reads committed source through the repository tools', { skip: !enabled && 'set TAKOMO_AGENT_LIVE_SMOKE=1' }, async t => {
  const state = process.env.TAKOMO_AGENT_STATE_DIR || join(homedir(), '.takomo-agent');
  const cwd = await mkdtemp(join(tmpdir(), 'takomo-live-smoke-'));
  t.after(() => rm(cwd, { recursive: true, force: true }));
  const git = args => execFileSync('git', args, { cwd, stdio: 'pipe' }).toString().trim();
  git(['init']);
  await writeFile(join(cwd, 'sample.js'), 'export function total(items, voucher) {\n  const sum = items.reduce((a, b) => a + b, 0);\n  return voucher ? sum - voucher : sum + sum;\n}\n');
  git(['add', '.']);
  git(['-c', 'user.name=Smoke', '-c', 'user.email=smoke@example.com', '-c', 'core.hooksPath=/dev/null', 'commit', '-m', 'fixture']);
  const revision = git(['rev-parse', 'HEAD']);
  const codex = new Codex({ executable: process.env.TAKOMO_CODEX_BIN || 'codex', cwd: join(state, 'workspace'), home: join(state, 'codex'), repositories: { smoke: cwd }, timeoutMs: 180_000, kind: 'bug_research' });
  try {
    const result = await codex.run({ kind: 'bug_research', repository_ref: { repository: 'smoke', revision: 'HEAD' }, snapshot: JSON.stringify({ title: 'Checkout total doubles after voucher removal' }), prompt: 'Read sample.js with repository_read and report the defective line with file:line evidence.' });
    assert.equal(result.repository_revision, revision);
    assert.ok(result.evidence.inspected.length > 0, 'research must retrieve source through the repository tools');
    assert.equal(result.evidence.inspected[0].path, 'sample.js');
    assert.equal(result.evidence.runtime_reproduced, false);
    assert.match(result.message, /sample\.js/);
  } finally { codex.close(); }
});

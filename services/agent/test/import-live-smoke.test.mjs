// Explicit opt-in only: one small paid/provider-backed App Server turn.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir, homedir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { Codex } from '../codex.mjs';
import { IMPORT_KIND } from '../spec-import.mjs';

test('real App Server produces a scoped draft with retrieved source evidence', { skip: process.env.TAKOMO_IMPORT_LIVE_SMOKE !== '1' && 'set TAKOMO_IMPORT_LIVE_SMOKE=1' }, async t => {
  const repo = await mkdtemp(join(tmpdir(), 'takomo-import-live-'));
  t.after(() => rm(repo, { recursive: true, force: true }));
  const git = args => execFileSync('git', args, { cwd: repo, stdio: 'pipe' }).toString().trim();
  git(['init']);
  await writeFile(join(repo, 'total.js'), 'export function total(items) {\n  if (!items.length) throw new Error("Empty order");\n  return items.reduce((sum, item) => sum + item.price, 0);\n}\n');
  git(['add', '.']);
  git(['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.com', '-c', 'core.hooksPath=/dev/null', 'commit', '-m', 'fixture']);
  const state = process.env.TAKOMO_AGENT_STATE_DIR || join(homedir(), '.takomo-agent');
  const codex = new Codex({ executable: process.env.TAKOMO_CODEX_BIN || 'codex', home: join(state, 'codex'), cwd: join(state, 'workspace'), repositories: { fixture: repo }, kind: IMPORT_KIND, timeoutMs: 90_000 });
  try {
    const result = await codex.run({ kind: IMPORT_KIND, repository_ref: { repository: 'fixture', revision: git(['rev-parse','HEAD']), scope: { include: ['total.js'], exclude: [] } }, repository_limits: { max_files: 1, max_source_bytes: 1000, max_tool_calls: 4 }, max_sections: 2, prompt: 'Read total.js and draft its implemented calculation and empty-order behavior. Keep sources to individual ranges you read.' });
    assert.ok(result.draft.sections.length > 0 && result.draft.sections.length <= 2);
    assert.ok(result.evidence.inspected.some(source => source.path === 'total.js'));
    assert.equal(result.manifest.counts.selected_files, 1);
  } finally { codex.close(); }
});

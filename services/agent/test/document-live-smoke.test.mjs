// Opt-in provider check: isolated temporary Codex state, no Takomo queue writes.
// TAKOMO_AGENT_DOCUMENT_LIVE_SMOKE=1 node --test services/agent/test/document-live-smoke.test.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, copyFile, chmod, rm } from 'node:fs/promises';
import { tmpdir, homedir } from 'node:os';
import { join } from 'node:path';
import { once } from 'node:events';
import { Codex } from '../codex.mjs';
import { DOCUMENT_KIND } from '../document.mjs';

test('real document drafts resume across processes with fresh context', {
  skip: process.env.TAKOMO_AGENT_DOCUMENT_LIVE_SMOKE !== '1' && 'set TAKOMO_AGENT_DOCUMENT_LIVE_SMOKE=1',
  timeout: 180_000,
}, async () => {
  const source = process.env.TAKOMO_AGENT_STATE_DIR || join(homedir(), '.takomo-agent');
  const temporary = await mkdtemp(join(tmpdir(), 'takomo-document-live-'));
  const home = join(temporary, 'codex');
  const cwd = join(temporary, 'workspace');
  try {
    await mkdir(home, { mode: 0o700 });
    await mkdir(cwd, { mode: 0o700 });
    // Copy rather than symlink: a provider auth refresh must not alter the live home.
    await copyFile(join(source, 'codex', 'auth.json'), join(home, 'auth.json'));
    await chmod(join(home, 'auth.json'), 0o600);
    const run = async job => {
      const codex = new Codex({ executable: process.env.TAKOMO_CODEX_BIN || 'codex', cwd, home, kind: DOCUMENT_KIND, timeoutMs: 75_000 });
      try { return await codex.run(job); }
      finally {
        const exited = once(codex.child, 'exit');
        codex.close();
        if (codex.child.exitCode === null && codex.child.signalCode === null) await exited;
      }
    };
    const snapshot = (action, days) => JSON.stringify({
      kind: DOCUMENT_KIND, mindmap_id: 'fixture', title: 'Invoice fixture', action,
      scope: { whole_document: true, section_ids: [] },
      sections: [{ id: 'expiration', title: 'Expiration', notes: `Invoices expire after ${days} days.` }],
    });
    const first = await run({ kind: DOCUMENT_KIND, snapshot: snapshot('draft_tests', 30), prompt: 'Draft one concise acceptance test for invoice expiration. Label it as a draft. Remember the conversation marker amber-otter for my next question. Stay under 60 words.' });
    assert.match(first.message, /draft/i);
    assert.match(first.message, /30/);
    const second = await run({ kind: DOCUMENT_KIND, thread_id: first.thread_id, snapshot: snapshot('discuss', 45), prompt: 'What was the conversation marker I asked you to remember, and what is the current expiration deadline? Answer both in one sentence.' });
    assert.equal(second.thread_id, first.thread_id);
    assert.notEqual(second.turn_id, first.turn_id);
    assert.match(second.message, /amber-otter/);
    assert.match(second.message, /45/);
  } finally { await rm(temporary, { recursive: true, force: true }); }
});

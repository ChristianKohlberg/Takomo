// Opt-in: one isolated provider turn, no Takomo jobs or persistent worker changes.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, copyFile, chmod, rm } from 'node:fs/promises';
import { tmpdir, homedir } from 'node:os';
import { join } from 'node:path';
import { once } from 'node:events';
import { createHash } from 'node:crypto';
import { Codex } from '../codex.mjs';
import { CLASSIFICATION_KIND } from '../ticket-document-classification.mjs';

test('real classifier retrieves document evidence and returns a grounded structured proposal', {
  skip: process.env.TAKOMO_AGENT_CLASSIFICATION_LIVE_SMOKE !== '1' && 'set TAKOMO_AGENT_CLASSIFICATION_LIVE_SMOKE=1', timeout: 180_000,
}, async () => {
  const state = process.env.TAKOMO_AGENT_STATE_DIR || join(homedir(), '.takomo-agent');
  const temporary = await mkdtemp(join(tmpdir(), 'takomo-classification-live-'));
  const home = join(temporary, 'codex');
  const cwd = join(temporary, 'workspace');
  try {
    await mkdir(home, { mode: 0o700 });
    await mkdir(cwd, { mode: 0o700 });
    await copyFile(join(state, 'codex', 'auth.json'), join(home, 'auth.json'));
    await chmod(join(home, 'auth.json'), 0o600);
    const section = (id, title, notes) => ({ id, title, notes, parent_id: null, prose_xml: '', version: createHash('sha256').update(notes).digest('hex') });
    const snapshot = {
      kind: CLASSIFICATION_KIND, schema_version: 1,
      ticket: { id: 'fixture-1', project: 'fixture', title: 'Invoice expiration', body: 'Implement invoice expiration after 30 days as described in the specification.', parent: null, revision: 'fixture-revision' },
      document: { kind: 'document_workspace', schema_version: 2, mindmap_id: 'fixture-map', title: 'Billing', action: 'discuss', context: { mode: 'automatic', section_ids: [], pinned_section_ids: [], quote: null }, sections: [section('invoice', 'Invoice expiration', 'Invoices expire after 30 days.'), section('notifications', 'Email notifications', 'Send weekly email summaries of outstanding invoices.')] },
    };
    const codex = new Codex({ executable: process.env.TAKOMO_CODEX_BIN || 'codex', cwd, home, kind: CLASSIFICATION_KIND, timeoutMs: 120_000 });
    try {
      const result = await codex.run({ kind: CLASSIFICATION_KIND, snapshot: JSON.stringify(snapshot), prompt: 'Classify this ticket.' });
      assert.ok(codex.document.usedTools().includes('document_search'));
      assert.ok(codex.document.usedTools().includes('document_read'));
      assert.equal(result.proposal.candidates.length, 1);
      assert.equal(result.proposal.candidates[0].section_id, 'invoice');
      assert.equal(result.proposal.candidates[0].version, snapshot.document.sections[0].version);
      assert.ok(snapshot.document.sections[0].notes.includes(result.proposal.candidates[0].quote));
      assert.equal(result.proposal.no_match_reason, null);
      assert.ok(result.evidence.document.sources.some(source => source.section_id === 'invoice'));
    } finally {
      const exited = once(codex.child, 'exit');
      codex.close();
      if (codex.child.exitCode === null && codex.child.signalCode === null) await exited;
    }
  } finally { await rm(temporary, { recursive: true, force: true }); }
});

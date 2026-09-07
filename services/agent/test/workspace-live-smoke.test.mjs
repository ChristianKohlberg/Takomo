// Opt-in only. Three tiny provider turns; no Takomo API or live worker changes.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, copyFile, chmod, rm } from 'node:fs/promises';
import { tmpdir, homedir } from 'node:os';
import { join } from 'node:path';
import { once } from 'node:events';
import { createHash } from 'node:crypto';
import { Codex } from '../codex.mjs';
import { WORKSPACE_KIND } from '../document-workspace.mjs';

test('real legacy migration preserves conversation then resumes scoped dynamic document tools', {
  skip: process.env.TAKOMO_AGENT_WORKSPACE_LIVE_SMOKE !== '1' && 'set TAKOMO_AGENT_WORKSPACE_LIVE_SMOKE=1',
  timeout: 300_000,
}, async () => {
  const source = process.env.TAKOMO_AGENT_STATE_DIR || join(homedir(), '.takomo-agent');
  const temporary = await mkdtemp(join(tmpdir(), 'takomo-workspace-live-'));
  const home = join(temporary, 'codex');
  const cwd = join(temporary, 'workspace');
  try {
    await mkdir(home, { mode: 0o700 });
    await mkdir(cwd, { mode: 0o700 });
    await copyFile(join(source, 'codex', 'auth.json'), join(home, 'auth.json'));
    await chmod(join(home, 'auth.json'), 0o600);
    const run = async job => {
      const codex = new Codex({ executable: process.env.TAKOMO_CODEX_BIN || 'codex', cwd, home, kind: job.kind, timeoutMs: 90_000 });
      try { return { ...await codex.run(job), used_tools: codex.document?.usedTools() }; }
      finally {
        const exited = once(codex.child, 'exit');
        codex.close();
        if (codex.child.exitCode === null && codex.child.signalCode === null) await exited;
      }
    };
    const section = days => ({ id: 'invoice', title: 'Invoice deadline', notes: `Invoices expire after ${days} days.`, parent_id: null, prose_xml: '', version: createHash('sha256').update(String(days)).digest('hex') });
    const legacy = await run({ kind: 'document_chat', snapshot: JSON.stringify({ kind: 'document_chat', mindmap_id: 'fixture', title: 'Invoice fixture', action: 'discuss', scope: { whole_document: true, section_ids: [] }, sections: [section(30)] }), prompt: 'Remember conversation marker copper-robin for my next request. Reply in one short sentence.' });
    const snapshot = days => JSON.stringify({ kind: WORKSPACE_KIND, schema_version: 2, mindmap_id: 'fixture', title: 'Invoice fixture', action: 'draft_tests', context: { mode: 'whole_document', section_ids: [], pinned_section_ids: [] }, sections: [section(days)] });
    const migrated = await run({ kind: WORKSPACE_KIND, thread_id: legacy.thread_id, migrate_thread: true, snapshot: snapshot(30), prompt: 'First use all three document tools: outline, search for Invoice, and read section invoice. Then draft one brief acceptance test with a section citation. Include the conversation marker I previously asked you to remember. Under 70 words.' });
    assert.notEqual(migrated.thread_id, legacy.thread_id);
    assert.equal(migrated.evidence.document_migration.previous_thread_id, legacy.thread_id);
    assert.equal(migrated.evidence.document_migration.retained_turns, 1);
    assert.deepEqual(migrated.used_tools.sort(), ['document_outline', 'document_read', 'document_search']);
    assert.match(migrated.message, /copper-robin/);
    assert.match(migrated.message, /30/);
    assert.match(migrated.message, /takomo-section:invoice/);
    const resumed = await run({ kind: WORKSPACE_KIND, thread_id: migrated.thread_id, snapshot: snapshot(45), prompt: 'Read section invoice using document_read again, then state the current deadline and the earlier conversation marker in one short sentence.' });
    assert.equal(resumed.thread_id, migrated.thread_id);
    assert.ok(resumed.used_tools.includes('document_read'));
    assert.match(resumed.message, /45/);
    assert.match(resumed.message, /copper-robin/);
    assert.equal(resumed.evidence.document.coverage.complete, true);
  } finally { await rm(temporary, { recursive: true, force: true }); }
});

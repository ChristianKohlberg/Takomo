import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { openDocumentWorkspace, WORKSPACE_KIND, migrationTranscript } from '../document-workspace.mjs';
import { Codex, configArgs, documentRestrictions, restrictions } from '../codex.mjs';
import { executeJob, ApiError } from '../service.mjs';

export function section(id, title, notes, extra = {}) {
  return { id, title, notes, parent_id: null, prose_xml: '', version: createHash('sha256').update(notes).digest('hex'), ...extra };
}
export function fixture(context = { mode: 'automatic', section_ids: [], pinned_section_ids: [] }, sections = [section('a', 'Invoices', 'Pay within 30 days.'), section('b', 'Überweisung Straße', 'Zahlungen dürfen verspätet eintreffen.')]) {
  return { kind: WORKSPACE_KIND, schema_version: 2, mindmap_id: 'map', title: 'Billing', action: 'discuss', context, sections };
}
const job = snapshot => ({ id: 'job', kind: WORKSPACE_KIND, prompt: 'Discuss payment', snapshot: JSON.stringify(snapshot) });
const read = async (workspace, id, offset, limit) => JSON.parse(await workspace.call('document_read', { section_id: id, offset, limit }));

test('selected scope fences outline, lexical search and reads; pins explicitly extend scope', async () => {
  const workspace = openDocumentWorkspace(job(fixture({ mode: 'selected', section_ids: ['a'], pinned_section_ids: ['b'] }, [section('a', 'Selected', 'Allowed'), section('b', 'Pinned', 'Context'), section('secret', 'Hidden', 'OUTSIDE')] )));
  assert.equal(JSON.parse(await workspace.call('document_outline', {})).total_sections, 2);
  assert.equal(JSON.parse(await workspace.call('document_search', { query: 'OUTSIDE' })).matches.length, 0);
  await assert.rejects(read(workspace, 'secret'), /outside the current document scope/);
  assert.equal((await read(workspace, 'b')).content, 'Context');
  assert.equal(workspace.progress().document.coverage.total_sections, 2);
});

test('search folds German/English accents and case, boosts headings and preserves explicit truncation', async () => {
  const workspace = openDocumentWorkspace(job(fixture(undefined, [section('body', 'Elsewhere', 'überweisung Straße'), section('heading', 'Überweisung Straße', 'A detail'), section('third', 'A third', 'Uberweisung strasse')])));
  const result = JSON.parse(await workspace.call('document_search', { query: 'UBERWEISUNG STRASSE', limit: 2 }));
  assert.equal(result.matches[0].section_id, 'heading');
  assert.equal(result.total_matches, 3);
  assert.equal(result.truncated, true);
  assert.deepEqual(workspace.progress().document.coverage.read_section_ids, []);
  assert.equal(workspace.progress().document.sources.length, 2);
});

test('pins-only selection remains bounded and accent expansion preserves matching excerpts', async () => {
  const workspace = openDocumentWorkspace(job(fixture({ mode: 'selected', section_ids: [], pinned_section_ids: ['pinned'] }, [section('outside', 'Outside', 'hidden'), section('pinned', 'Pinned', `${'ß'.repeat(2000)} Invoice overdue`)])));
  const result = JSON.parse(await workspace.call('document_search', { query: 'Invoice' }));
  assert.match(result.matches[0].excerpt, /Invoice overdue/);
  assert.equal(JSON.parse(await workspace.call('document_outline', {})).total_sections, 1);
});

test('paginated reads track complete contiguous coverage, not snippets, gaps or duplicate reads', async () => {
  const workspace = openDocumentWorkspace(job(fixture(undefined, [section('large', 'Large', 'x'.repeat(24_001))])));
  const start = await read(workspace, 'large', 0, 12_000);
  assert.equal(start.next_offset, 12_000);
  await read(workspace, 'large', 24_000, 12_000);
  await read(workspace, 'large', 0, 12_000);
  assert.equal(workspace.progress().document.coverage.complete, false);
  await read(workspace, 'large', 12_000, 12_000);
  assert.deepEqual(workspace.progress().document.coverage, { read_section_ids: ['large'], total_sections: 1, complete: true });
});

test('rich content is preserved and refreshed workspaces cannot see previous versions', async () => {
  const original = fixture(undefined, [section('a', 'Table', 'Old', { prose_xml: '<table><row><cell>37</cell></row></table>' })]);
  const workspace = openDocumentWorkspace(job(original));
  assert.match((await read(workspace, 'a')).content, /<table>/);
  const refreshed = openDocumentWorkspace(job(fixture(undefined, [section('b', 'New', 'Only new contents')])));
  await assert.rejects(read(refreshed, 'a'), /outside/);
});

test('initial retrieval prioritizes pins and selection without silently sending the entire document', () => {
  const sections = Array.from({ length: 20 }, (_, i) => section(`s${i}`, `Heading ${i}`, `Content ${i}`));
  const workspace = openDocumentWorkspace(job(fixture({ mode: 'automatic', section_ids: [], pinned_section_ids: ['s19'] }, sections)));
  const input = workspace.input('Discuss');
  const data = JSON.parse(input.split('CURRENT DOCUMENT CONTEXT (immutable reference; replaces earlier sources):\n')[1].split('\nUse document tools')[0]);
  assert.equal(data.initial_sources[0].section_id, 's19');
  assert.ok(data.initial_sources.length < sections.length);
  assert.equal(workspace.progress().document.coverage.complete, false);
});

test('tool arguments, source shape and resource limits fail closed', async () => {
  assert.throws(() => openDocumentWorkspace(job(fixture({ mode: 'selected', section_ids: [], pinned_section_ids: [] }))), /selection/);
  const workspace = openDocumentWorkspace(job(fixture()));
  await assert.rejects(workspace.call('document_read', { section_id: 'a', path: '/etc/passwd' }), /arguments/);
  await assert.rejects(workspace.call('repository_read', { path: '/etc/passwd' }), /Unsupported/);
  await assert.rejects(workspace.call('document_search', { query: 'x'.repeat(301) }), /literal query/);
  await assert.rejects(read(workspace, 'a', -1), /integer/);
  await assert.rejects(read(workspace, 'a', 0, 0), /positive/);
  for (let i = 0; i < 200; i++) await workspace.call('document_outline', {}).catch(() => {});
  await assert.rejects(workspace.call('document_outline', {}), /budget exhausted/);
});

test('cumulative content budget stops large reads with incomplete coverage', async () => {
  const workspace = openDocumentWorkspace(job(fixture(undefined, [section('large', 'Large', 'x'.repeat(900_000))])));
  let offset = 0;
  let failure;
  while (offset !== null) {
    try { offset = (await read(workspace, 'large', offset, 12_000)).next_offset; }
    catch (error) { failure = error; break; }
  }
  assert.match(failure.message, /content budget exhausted/);
  assert.equal(workspace.progress().document.coverage.complete, false);
});

test('migration imports visible complete conversation turns only, bounded with explicit omissions', () => {
  const transcript = migrationTranscript({ turns: [
    { status: 'completed', items: [{ type: 'userMessage', content: [{ type: 'text', text: 'CURRENT ACTION: discuss\nSTALE SOURCE SNAPSHOT\n\nUSER MESSAGE:\nRemember the marker' }] }, { type: 'agentMessage', phase: 'final_answer', text: 'Remembered' }, { type: 'agentMessage', phase: 'commentary', text: 'PRIVATE COMMENTARY' }, { type: 'reasoning', text: 'PRIVATE REASONING' }, { type: 'dynamicToolCall', text: 'PRIVATE TOOL' }] },
    { status: 'completed', items: [{ type: 'agentMessage', text: 'x'.repeat(200_001) }] },
    { status: 'failed', items: [{ type: 'agentMessage', text: 'FAILED TURN' }] },
  ] });
  assert.equal(transcript.retained_turns, 1);
  assert.equal(transcript.omitted_turns, 2);
  assert.match(transcript.text, /Remember the marker/);
  assert.doesNotMatch(transcript.text, /PRIVATE|FAILED|STALE SOURCE/);
});

const createCodex = () => new Codex({ kind: WORKSPACE_KIND, executable: process.execPath, args: [fileURLToPath(new URL('./fake-workspace.mjs', import.meta.url)), ...configArgs(documentRestrictions)], cwd: '/tmp', home: '/tmp', timeoutMs: 1000 });
test('dynamic tool protocol retrieves scoped evidence and resumes with fresh tools source', async () => {
  for (const thread_id of [undefined, 'workspace-thread']) {
    const codex = createCodex();
    try {
      const result = await codex.run({ ...job(fixture()), thread_id });
      assert.equal(result.thread_id, 'workspace-thread');
      assert.deepEqual(codex.document.usedTools().sort(), ['document_outline', 'document_read', 'document_search']);
      assert.equal(result.evidence.document.coverage.complete, true);
      assert.match(result.message, /Pay within 30 days/);
    } finally { codex.close(); }
  }
});

test('migration starts a tool thread and publishes rebinding metadata before the turn', async () => {
  const codex = createCodex();
  const sessions = [];
  try {
    const result = await codex.run({ ...job(fixture()), thread_id: 'legacy-thread', migrate_thread: true }, async ids => sessions.push(ids));
    assert.deepEqual(sessions[0].evidence.document_migration, { previous_thread_id: 'legacy-thread', new_thread_id: 'workspace-thread', retained_turns: 1, omitted_turns: 0 });
    assert.deepEqual(result.evidence.document_migration, sessions[0].evidence.document_migration);
    assert.match(result.message, /legacy-marker/);
  } finally { codex.close(); }
});

test('workspace policy cannot run on a legacy no-tools process', async () => {
  const codex = new Codex({ kind: 'document_chat', executable: process.execPath, args: [fileURLToPath(new URL('./fake-codex.mjs', import.meta.url)), ...configArgs(restrictions)], cwd: '/tmp', home: '/tmp' });
  try { await assert.rejects(codex.run(job(fixture())), /policy|cannot run/); }
  finally { codex.close(); }
});

test('document dynamic dispatch returns a tool error for repository access without executing it', async () => {
  const codex = createCodex();
  try {
    const result = await codex.run({ ...job(fixture()), prompt: 'UNSUPPORTED_REPOSITORY_TOOL' });
    assert.match(result.message, /Unsupported document tool/);
    assert.equal(codex.repository, undefined);
    assert.deepEqual(codex.document.usedTools(), []);
  } finally { codex.close(); }
});

test('lease failure prevents publishing workspace content or migration rebinding after ownership loss', async () => {
  let delivered = false;
  await executeJob({ ...job(fixture()), thread_id: 'legacy-thread', migrate_thread: true }, {
    serviceId: 'worker', signal: new AbortController().signal, createCodex,
    api: async path => { if (path.endsWith('/heartbeat')) throw new ApiError(409); delivered = true; },
  });
  assert.equal(delivered, false);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { Codex, configArgs, classificationRestrictions } from '../codex.mjs';
import { CLASSIFICATION_KIND, classificationSnapshot, parseClassificationProposal } from '../ticket-document-classification.mjs';
import { openDocumentWorkspace } from '../document-workspace.mjs';
import { executeJob, ApiError, supportedKinds } from '../service.mjs';

const notes = 'Invoices expire after 30 days.';
const version = createHash('sha256').update(notes).digest('hex');
const source = (body = '') => ({ kind: CLASSIFICATION_KIND, schema_version: 1,
  ticket: { id: 'ticket-1', project: 'demo', title: 'Invoice expiration', body: `Implement the invoice deadline. ${body}`, parent: 'epic-1', revision: 'revision-1' },
  document: { kind: 'document_workspace', schema_version: 2, mindmap_id: 'map', title: 'Billing', action: 'discuss', context: { mode: 'automatic', section_ids: [], pinned_section_ids: [], quote: null }, sections: [{ id: 'invoice', parent_id: null, title: 'Invoice expiration', notes, prose_xml: '', version }] },
});
const job = mode => ({ id: 'classification-job', attempt_id: 'attempt', kind: CLASSIFICATION_KIND, snapshot: JSON.stringify(source(mode)), prompt: 'Classify the ticket.' });
const createCodex = () => new Codex({ kind: CLASSIFICATION_KIND, executable: process.execPath, args: [fileURLToPath(new URL('./fake-classification.mjs', import.meta.url)), ...configArgs(classificationRestrictions)], cwd: '/tmp', home: '/tmp', timeoutMs: 1500 });

test('classification combines scoped document tools and structured output without applying relationships', async () => {
  const codex = createCodex();
  try {
    const result = await codex.run(job(''));
    assert.deepEqual(codex.document.usedTools().sort(), ['document_outline', 'document_read', 'document_search']);
    assert.equal(result.proposal.candidates[0].section_id, 'invoice');
    assert.equal(result.proposal.candidates[0].version, version);
    assert.equal(result.proposal.candidates[0].quote, notes);
    assert.match(result.message, /for review/);
    assert.deepEqual(result.evidence.document.sources, [{ section_id: 'invoice', version }]);
    assert.equal(result.evidence.document.coverage.complete, true);
    assert.ok(supportedKinds.includes(CLASSIFICATION_KIND));
  } finally { codex.close(); }
});
for (const mode of ['NO_MATCH', 'AMBIGUOUS']) {
  test(`classification preserves explicit ${mode}`, async () => {
    const codex = createCodex();
    try {
      const result = await codex.run(job(mode));
      if (mode === 'NO_MATCH') { assert.deepEqual(result.proposal.candidates, []); assert.ok(result.proposal.no_match_reason); }
      else assert.ok(result.proposal.ambiguity);
    } finally { codex.close(); }
  });
}
for (const mode of ['UNKNOWN_SECTION', 'STALE_VERSION', 'BAD_QUOTE', 'DUPLICATE', 'CONFIDENCE', 'INVALID_JSON']) {
  test(`classification rejects ${mode} instead of repairing or applying it`, async () => {
    const codex = createCodex();
    try { await assert.rejects(codex.run(job(mode))); }
    finally { codex.close(); }
  });
}

test('classification cannot reuse another session and rejects malformed scope', () => {
  assert.throws(() => classificationSnapshot({ ...job(''), thread_id: 'existing' }), /standalone thread/);
  const snapshot = source();
  snapshot.document.context.mode = 'selected';
  assert.throws(() => classificationSnapshot({ ...job(''), snapshot: JSON.stringify(snapshot) }), /scope/);
});

test('classification accepts the full supported ticket body size and rejects oversize content', () => {
  const snapshot = source();
  snapshot.ticket.body = 'x'.repeat(131_072);
  assert.equal(classificationSnapshot({ ...job(''), snapshot: JSON.stringify(snapshot) }).ticket.body.length, 131_072);
  snapshot.ticket.body += 'x';
  assert.throws(() => classificationSnapshot({ ...job(''), snapshot: JSON.stringify(snapshot) }), /body/);
});

test('quotes must have been read, including across contiguous pages, not merely exist in the snapshot or a search hit', async () => {
  const snapshot = source();
  snapshot.document.sections[0].notes = `Prefix ${'x'.repeat(6000)} Invoices expire after 30 days.`;
  const document = openDocumentWorkspace({ snapshot: JSON.stringify(snapshot.document) });
  document.input('Invoice expiration');
  await document.call('document_search', { query: 'Invoices expire' });
  const proposal = { candidates: [{ section_id: 'invoice', version, quote: notes, rationale: 'Matching behavior.' }], ambiguity: null, no_match_reason: null };
  assert.throws(() => parseClassificationProposal(JSON.stringify(proposal), snapshot, document), /not retrieved/);
  await document.call('document_read', { section_id: 'invoice', offset: 6000, limit: 15 });
  await document.call('document_read', { section_id: 'invoice', offset: 6015, limit: 100 });
  assert.equal(parseClassificationProposal(JSON.stringify(proposal), snapshot, document).candidates.length, 1);
});

test('unread gaps cannot be normalized away to authorize a longer quotation', async () => {
  const snapshot = source();
  snapshot.document.sections[0].notes = 'Invoice\n\n\nexpires';
  const document = openDocumentWorkspace({ snapshot: JSON.stringify(snapshot.document) });
  await document.call('document_read', { section_id: 'invoice', offset: 0, limit: 7 });
  await document.call('document_read', { section_id: 'invoice', offset: 10, limit: 20 });
  assert.equal(document.hasReadQuote('invoice', 'Invoice expires'), false);
  await document.call('document_read', { section_id: 'invoice', offset: 7, limit: 3 });
  assert.equal(document.hasReadQuote('invoice', 'Invoice expires'), true);
});

test('classification delivery retries preserve the proposal without running another provider turn', async () => {
  let runs = 0;
  const results = [];
  await executeJob(job(''), {
    serviceId: 'worker', signal: new AbortController().signal,
    createCodex: () => { runs++; return createCodex(); },
    api: async (path, body) => { if (path.endsWith('/result')) { results.push(body); if (results.length === 1) throw new ApiError(503); } return {}; },
  });
  assert.equal(runs, 1);
  assert.equal(results.length, 2);
  assert.deepEqual(results[0], results[1]);
  assert.equal(results[0].proposal.candidates[0].section_id, 'invoice');
});

test('classification cannot publish a proposal after losing its lease', async () => {
  let delivered = false;
  await executeJob(job(''), {
    serviceId: 'worker', signal: new AbortController().signal, createCodex,
    api: async path => { if (path.endsWith('/heartbeat')) throw new ApiError(409); delivered = true; },
  });
  assert.equal(delivered, false);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { Codex, configArgs, restrictions, profileFor } from '../codex.mjs';
import { executeJob, ApiError } from '../service.mjs';
import { ORGANIZER_KIND, organizerSnapshot, parseOrganizerProposal, organizerSchema } from '../organizer.mjs';

const snapshot = {
  tickets: [{ id: 'jeans-1', title: 'Seam durability', body: 'Use the supplied wash test.' }, { id: 'jeans-2', title: 'Waist fit' }, { id: 'jeans-3', title: 'Seam strength follow-up' }],
  lanes: [{ id: 'lane-fabric', title: 'Fabric construction', purpose: 'Develop a durable fabric assembly', context: 'Existing project convention.' }],
  specification: { representation: 'persisted_projection', text: 'Use denim; test shrinkage after washing.' },
};
const job = { id: 'job-organize', attempt_id: 'attempt-1', kind: ORGANIZER_KIND, project: 'jeans', snapshot: JSON.stringify(snapshot), prompt: 'Organize the pending tickets without inventing missing constraints.' };
function createCodex() {
  return new Codex({ executable: process.execPath, args: [fileURLToPath(new URL('./fake-organizer.mjs', import.meta.url)), 'app-server', '--stdio', ...configArgs(restrictions)], cwd: '/tmp', home: '/tmp', timeoutMs: 500, kind: ORGANIZER_KIND });
}
function proposal() {
  return { groups: [{ lane_id: 'lane-fabric', title: 'Fabric construction', purpose: 'Develop a durable fabric assembly', context: 'Proposed context', readiness: 'needs_clarification', reason: 'Confirm wash threshold.', ticket_ids: ['jeans-1', 'jeans-2'] }], unassigned: [{ ticket_id: 'jeans-3', reason: 'Potential duplicate of jeans-1.' }] };
}

test('organizer uses schema-constrained no-tools turns and resumes only the supplied organizer thread', async () => {
  assert.equal(profileFor(ORGANIZER_KIND), restrictions);
  for (const thread_id of [undefined, 'earlier-organizer-thread']) {
    const codex = createCodex(), sessions = [];
    try {
      const result = await codex.run({ ...job, thread_id }, async value => sessions.push(value));
      assert.equal(result.thread_id, thread_id || 'organizer-thread');
      assert.equal(result.turn_id, 'organizer-turn');
      assert.equal(result.proposal.groups[0].lane_id, 'lane-fabric');
      assert.equal(result.proposal.groups[1].readiness, 'needs_clarification');
      assert.match(result.proposal.unassigned[0].reason, /duplicate of jeans-1/);
      assert.match(result.message, /1 existing and 1 new lanes/);
      assert.match(result.message, /No lanes or tickets have been changed/);
      assert.equal(sessions.length, 2);
      assert.equal(result.repository_revision, undefined);
      assert.equal(codex.repository, undefined);
    } finally { codex.close(); }
  }
});

test('organizer allows bounded proposals over the section message limit but returns a short summary', async () => {
  const codex = createCodex();
  try {
    const result = await codex.run({ ...job, prompt: 'BIG_VALID' });
    assert(Buffer.byteLength(JSON.stringify(result.proposal)) > 64_000);
    assert(Buffer.byteLength(result.message) < 1000);
  } finally { codex.close(); }
});

for (const prompt of ['INVALID_JSON', 'UNKNOWN_TICKET', 'RENAMED_LANE', 'OVERSIZED', 'FAILED_TURN', 'UNSUPPORTED_TOOL', 'HANG']) {
  test(`organizer rejects ${prompt} instead of applying or retrying malformed work`, async () => {
    let result, starts = 0;
    await executeJob({ ...job, prompt }, {
      serviceId: 'worker', signal: new AbortController().signal,
      createCodex: () => { starts++; return createCodex(); },
      api: async (path, body) => { if (path.endsWith('/result')) result = body; return {}; },
    });
    assert.equal(starts, 1);
    assert.equal(result.status, 'failed');
    assert.equal(result.proposal, undefined);
    assert(result.error);
  });
}

test('valid organizer proposal delivery retains scope/fence and retries delivery without another model turn', async () => {
  let starts = 0; const deliveries = [], paths = [];
  await executeJob(job, {
    serviceId: 'worker', signal: new AbortController().signal,
    createCodex: () => { starts++; return createCodex(); },
    api: async (path, body) => {
      paths.push(path);
      if (path.endsWith('/result')) { deliveries.push(body); if (deliveries.length === 1) throw new ApiError(503); }
      return {};
    },
  });
  assert.equal(starts, 1);
  assert.equal(deliveries.length, 2);
  assert.deepEqual(deliveries[0], deliveries[1]);
  assert.equal(deliveries[0].attempt_id, job.attempt_id);
  assert.equal(deliveries[0].status, 'completed');
  assert(deliveries[0].proposal.groups.length);
  assert(paths.every(path => path.startsWith(`/v1/agent-jobs/${job.id}/`)));
});

test('organizer heartbeat loss stops work and cannot deliver a stale proposal', async () => {
  let delivered = false;
  await executeJob(job, {
    serviceId: 'worker', signal: new AbortController().signal, createCodex,
    api: async path => { if (path.endsWith('/heartbeat')) throw new ApiError(409); delivered = true; },
  });
  assert.equal(delivered, false);
});

test('proposal validation rejects scope drift, omissions, unsupported fields and byte overflow', () => {
  const valid = proposal();
  assert.deepEqual(parseOrganizerProposal(JSON.stringify(valid), snapshot), valid);
  const invalid = mutate => {
    const value = structuredClone(valid); mutate(value);
    assert.throws(() => parseOrganizerProposal(JSON.stringify(value), snapshot));
  };
  invalid(value => { value.groups[0].ticket_ids.push('unknown'); });
  invalid(value => { value.groups[0].ticket_ids.push('jeans-3'); });
  invalid(value => { value.unassigned = []; });
  invalid(value => { value.groups[0].ticket_ids = []; });
  invalid(value => { value.groups[0].lane_id = 'other-project-lane'; });
  invalid(value => { value.groups[0].purpose = 'Changed purpose'; });
  invalid(value => { value.groups[0].readiness = 'implement_now'; });
  invalid(value => { value.groups[0].reason = ' '; });
  invalid(value => { value.groups[0].shell_command = 'touch unwanted'; });
  invalid(value => { value.groups[0].context = '🧵'.repeat(16001); });
  invalid(value => { value.unassigned[0].reason = '🧵'.repeat(1001); });
  invalid(value => { value.groups.push({ ...value.groups[0], ticket_ids: ['jeans-3'] }); value.unassigned = []; });
  assert.equal(organizerSchema.additionalProperties, false);
});

test('organizer snapshot validation rejects ambiguous or oversized input before model work', async () => {
  assert.deepEqual(organizerSnapshot(job.snapshot), snapshot);
  assert.throws(() => organizerSnapshot('not JSON'));
  assert.throws(() => organizerSnapshot(JSON.stringify({ tickets: [{ id: 'a' }, { id: 'a' }], lanes: [] })));
  assert.throws(() => organizerSnapshot(JSON.stringify({ tickets: [], lanes: Array.from({ length: 101 }, (_, i) => ({ id: String(i) })) })));
  assert.throws(() => organizerSnapshot('x'.repeat(512001)));
  const codex = createCodex();
  try { await assert.rejects(codex.run({ ...job, snapshot: 'invalid' }), /snapshot/); }
  finally { codex.close(); }
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Codex, configArgs, restrictions } from '../codex.mjs';
import { documentInput, DOCUMENT_KIND } from '../document.mjs';
import { executeJob, ApiError, supportedKinds } from '../service.mjs';

const snapshot = (action, whole = false) => JSON.stringify({
  kind: DOCUMENT_KIND, mindmap_id: 'map-1', title: 'Billing', action,
  scope: { whole_document: whole, section_ids: whole ? [] : ['one', 'two'] },
  sections: [
    { id: 'one', parent_id: null, title: 'Expiration', notes: 'Invoices expire after 30 days.', prose_xml: '<codeblock language="javascript">assert.equal(days, 30)</codeblock>' },
    { id: 'two', title: 'Reminders', notes: 'Send reminders on day 20.' },
  ],
});
const job = { kind: DOCUMENT_KIND, id: 'document-job', attempt_id: 'attempt-1', prompt: 'DOCUMENT_PROTOCOL', snapshot: snapshot('discuss') };
const createCodex = (kind = DOCUMENT_KIND) => new Codex({
  kind, executable: process.execPath,
  args: [fileURLToPath(new URL('./fake-codex.mjs', import.meta.url)), ...configArgs(restrictions)],
  cwd: '/tmp', home: '/tmp', timeoutMs: 2000,
});

for (const action of ['discuss', 'grill', 'draft_tests', 'draft_questions']) {
  test(`${action} delivers both selected sections with draft-only, no-tools policy`, async () => {
    const codex = createCodex();
    try {
      const response = JSON.parse((await codex.run({ ...job, snapshot: snapshot(action) })).message);
      assert.equal(response.thread.method, 'thread/start');
      assert.match(response.thread.baseInstructions, /Test plans, illustrative test code and questions are drafts only/);
      assert.doesNotMatch(response.thread.baseInstructions, /or create tests/);
      assert.equal(response.thread.dynamicTools, undefined);
      assert.equal(response.turn.approvalPolicy, 'never');
      assert.deepEqual(response.turn.sandboxPolicy, { type: 'readOnly', networkAccess: false });
      assert.match(response.turn.input[0].text, new RegExp(`CURRENT ACTION: ${action}`));
      assert.ok(response.turn.input[0].text.includes(snapshot(action)));
      assert.equal(response.turn.outputSchema, undefined);
    } finally { codex.close(); }
  });
}

test('whole-document follow-up resumes the persisted thread and supplies fresh scope/content', async () => {
  const codex = createCodex();
  const current = JSON.parse(snapshot('discuss', true));
  current.sections[0].notes = 'Invoices now expire after 45 days.';
  const sessions = [];
  try {
    const response = JSON.parse((await codex.run({ ...job, thread_id: 'persisted-thread', snapshot: JSON.stringify(current) }, async ids => sessions.push(ids))).message);
    assert.equal(response.thread.method, 'thread/resume');
    assert.equal(response.thread.threadId, 'persisted-thread');
    assert.match(response.thread.developerInstructions, /scope and content replace earlier snapshots/);
    assert.match(response.turn.input[0].text, /45 days/);
    assert.doesNotMatch(response.turn.input[0].text, /30 days/);
    assert.deepEqual(sessions, [{ thread_id: 'persisted-thread' }, { thread_id: 'persisted-thread', turn_id: 'turn-1' }]);
  } finally { codex.close(); }
});

test('rejects unknown actions, invalid scope and another process policy before starting a thread', async () => {
  assert.throws(() => documentInput({ ...job, snapshot: snapshot('execute_tests') }), /unsupported action/);
  const invalid = JSON.parse(snapshot('discuss'));
  invalid.scope.section_ids = ['one'];
  assert.throws(() => documentInput({ ...job, snapshot: JSON.stringify(invalid) }), /match its scope/);
  invalid.scope.whole_document = true;
  assert.throws(() => documentInput({ ...job, snapshot: JSON.stringify(invalid) }), /match its scope/);
  const codex = createCodex('section_chat');
  try { await assert.rejects(codex.run(job), /own Codex process policy/); }
  finally { codex.close(); }
  assert.ok(supportedKinds.includes(DOCUMENT_KIND));
});

test('document tool requests fail closed', async () => {
  const codex = createCodex();
  try { await assert.rejects(codex.run({ ...job, prompt: 'TOOL' }), /unsupported tool or approval/); }
  finally { codex.close(); }
});

test('document result retry preserves one turn; lease loss cannot publish its draft', async () => {
  for (const loseLease of [false, true]) {
    let runs = 0;
    const deliveries = [];
    await executeJob(job, {
      serviceId: 'worker', signal: new AbortController().signal,
      createCodex: () => ({ close() {}, async run(_job, session) {
        runs++;
        await session({ thread_id: 'persisted-thread', turn_id: 'turn-2' });
        return { message: 'Draft tests', thread_id: 'persisted-thread', turn_id: 'turn-2' };
      } }),
      api: async (path, body) => {
        if (path.endsWith('/heartbeat') && loseLease) throw new ApiError(409);
        if (path.endsWith('/result')) {
          deliveries.push(body);
          if (deliveries.length === 1) throw new ApiError(503);
        }
        return {};
      },
    });
    assert.equal(runs, 1);
    assert.equal(deliveries.length, loseLease ? 0 : 2);
    if (!loseLease) assert.deepEqual(deliveries[0], deliveries[1]);
  }
});

test('service claims advertise document support alongside all legacy job kinds', async () => {
  const state = await mkdtemp(join(tmpdir(), 'takomo-document-worker-'));
  let claimed;
  const server = createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    claimed = { path: req.url, body: JSON.parse(Buffer.concat(chunks).toString()) };
    res.setHeader('Content-Type', 'application/json');
    res.end(JSON.stringify({ job: null }));
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const child = spawn(process.execPath, [fileURLToPath(new URL('../service.mjs', import.meta.url)), '--once'], {
    env: { PATH: process.env.PATH, TAKOMO_URL: `http://127.0.0.1:${server.address().port}`, TAKOMO_AGENT_TOKEN: 'test-only', TAKOMO_AGENT_STATE_DIR: state, TAKOMO_AGENT_SERVICE_ID: 'document-worker' },
    stdio: 'ignore',
  });
  try {
    const [code] = await once(child, 'exit', { signal: AbortSignal.timeout(5000) });
    assert.equal(code, 0);
    assert.equal(claimed.path, '/v1/agent-jobs/claim');
    assert.deepEqual(claimed.body, { service_id: 'document-worker', wait_seconds: 0, supported_kinds: ['section_chat', 'bug_research', 'lane_organize', 'document_chat', 'document_workspace', 'ticket_document_classify'] });
  } finally {
    child.kill();
    await new Promise(resolve => server.close(resolve));
    await rm(state, { recursive: true, force: true });
  }
});

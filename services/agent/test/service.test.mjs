import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { Codex, configArgs, validateConfig, restrictions } from '../codex.mjs';
import { executeJob, ApiError } from '../service.mjs';
const createCodex = () => new Codex({ executable: process.execPath, args: [fileURLToPath(new URL('./fake-codex.mjs', import.meta.url)), 'app-server', '--stdio', ...configArgs(restrictions)], cwd: '/tmp', home: '/tmp', timeoutMs: 2000 });
const job = { id: 'job-1', attempt_id: 'attempt-1', prompt: 'Grill', snapshot: 'Invoices expire soon.' };

test('starts and resumes threads, persists IDs early, extracts final text once', async () => {
  for (const thread_id of [undefined, 'thread-existing']) {
    const codex = createCodex();
    const sessions = [];
    try {
      const response = await codex.run({ ...job, thread_id }, async ids => sessions.push(ids));
      assert.equal(response.message, `Which deadline applies? (${thread_id || 'thread-new'})`);
      assert.equal(response.turn_id, 'turn-1');
      assert.deepEqual(sessions, [{ thread_id: thread_id || 'thread-new' }, { thread_id: thread_id || 'thread-new', turn_id: 'turn-1' }]);
    } finally { codex.close(); }
  }
});
for (const prompt of ['FAIL', 'EXIT', 'TOOL', 'HANG']) {
  test(`fails safely for ${prompt}`, async () => {
    const codex = createCodex();
    try { await assert.rejects(codex.run({ ...job, prompt })); }
    finally { codex.close(); }
  });
}
test('result delivery retry does not reexecute Codex', async () => {
  let runs = 0;
  let deliveries = 0;
  const results = [];
  await executeJob(job, {
    serviceId: 'worker', signal: new AbortController().signal,
    createCodex: () => ({ close() {}, async run(_job, ids) { runs++; await ids({ thread_id: 'thread-new' }); return { message: 'Question?', thread_id: 'thread-new', turn_id: 'turn-1' }; } }),
    api: async (path, body) => {
      if (path.endsWith('/result')) { results.push(body); if (++deliveries === 1) throw new ApiError(503); }
      return {};
    },
  });
  assert.equal(runs, 1);
  assert.equal(deliveries, 2);
  assert.deepEqual(results[0], results[1]);
  assert.equal(results[0].attempt_id, 'attempt-1');
});
test('heartbeat failure terminates Codex and never delivers stale output', async () => {
  let closed = false;
  let delivered = false;
  await executeJob(job, {
    serviceId: 'worker', signal: new AbortController().signal,
    createCodex: () => ({ close() { closed = true; }, async run(_job, ids) { await ids({ thread_id: 'thread-new' }); return { message: 'late' }; } }),
    api: async path => { if (path.endsWith('/heartbeat')) throw new ApiError(409); delivered = true; },
  });
  assert.equal(closed, true);
  assert.equal(delivered, false);
});

test('effective configuration rejects inherited MCP, plugins, and enabled tools', () => {
  validateConfig(restrictions);
  assert.throws(() => validateConfig({ ...restrictions, mcp_servers: { hidden: { command: 'danger' } } }));
  assert.throws(() => validateConfig({ ...restrictions, plugins: { hidden: { enabled: true } } }));
  assert.throws(() => validateConfig({ ...restrictions, features: { ...restrictions.features, shell_tool: true } }));
  assert.throws(() => validateConfig({ ...restrictions, web_search: 'live' }));
  assert.throws(() => validateConfig({ ...restrictions, features: { ...restrictions.features, code_mode_host: true } }), /disable code_mode_host/);
});

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
const cli = fileURLToPath(new URL('../takomo', import.meta.url));

test('reopen routes a ticket reopen with its reason and a question reopen by id shape', async () => {
  const requests = [];
  const server = createServer(async (req, res) => {
    let body = '';
    for await (const c of req) body += c;
    requests.push({ method: req.method, path: req.url, body: body ? JSON.parse(body) : null });
    res.setHeader('Content-Type', 'application/json');
    if (req.method === 'GET') return res.end(JSON.stringify({ id: 'demo-ab12', state: 'done', project: 'demo' }));
    if (req.url.startsWith('/v1/questions/')) return res.end(JSON.stringify({ question: { id: 'q-abcd1234' }, ticket: { id: 'demo-ab12', state: 'needs-decision' } }));
    res.end(JSON.stringify({ id: 'demo-ab12', state: 'brief', project: 'demo' }));
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const home = mkdtempSync(join(tmpdir(), 'takomo-cli-reopen-'));
  const run = (args) => new Promise((resolve, reject) => {
    const p = spawn('bash', [cli, ...args], { cwd: home, env: { ...process.env, HOME: home, TAKOMO_URL: `http://127.0.0.1:${server.address().port}/v1`, TAKOMO_TOKEN: 'local-test-only', TAKOMO_PROJECT: 'demo' } });
    let stdout = '', stderr = '';
    p.stdout.on('data', (x) => (stdout += x)); p.stderr.on('data', (x) => (stderr += x));
    p.on('error', reject); p.on('exit', (code) => (code === 0 ? resolve(stdout) : reject(Error(stderr))));
  });
  try {
    const out = await run(['reopen', 'demo-ab12', '--reason', 'closed by a script', '--to', 'brief']);
    assert.match(out, /demo-ab12 reopened: done -> brief/);
    const post = requests.find((r) => r.method === 'POST');
    assert.deepEqual(post, { method: 'POST', path: '/v1/tickets/demo-ab12/reopen', body: { reason: 'closed by a script', to: 'brief' } });

    requests.length = 0;
    await assert.rejects(run(['reopen', 'demo-ab12']), /--reason is required/);
    assert.equal(requests.length, 0, 'a ticket reopen without a reason never reaches the server');

    await run(['reopen', 'q-abcd1234']);
    assert.deepEqual(requests.map((r) => [r.method, r.path]), [['POST', '/v1/questions/q-abcd1234/reopen']]);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm, chmod } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { setTimeout as sleep } from 'node:timers/promises';
import { invocation, promptFor, providerEnvironment, runProvider, sessionId } from '../providers.mjs';
import { ApiError, apiClient, executeHandoff, finishWorkspace, git, loadConfig, lockWorkspace, prepareWorkspace, reviewEvidence, runNext } from '../worker.mjs';

const fake = fileURLToPath(new URL('./fake-provider.mjs', import.meta.url));
const job = { id: 'hf-test', project: 'demo', lane: 'lane-test', kind: 'preparation', provider: 'codex', attempt: 1,
  instructions: 'Organize these tickets', snapshot: { lane: { title: 'A lane', context: 'Project-specific context' }, tickets: [{ id: 'demo-1', title: 'User title $(touch /tmp/no)', body: 'Useful details' }] } };
async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'takomo-lane-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const cwd = join(root, 'workspace'), home = join(root, 'home'), state = join(root, 'state');
  for (const dir of [cwd, home, state]) await mkdir(dir);
  await git(cwd, ['init', '-b', 'lane-work']);
  await writeFile(join(cwd, 'README.md'), 'Initial\n');
  await git(cwd, ['add', '.']);
  await git(cwd, ['-c', 'user.name=Test', '-c', 'user.email=test@localhost', '-c', 'commit.gpgsign=false', 'commit', '-m', 'Initial']);
  return { root, cwd, home, state, config: { projects: { demo: cwd }, providers: { codex: { home }, claude: { home } } }, signal: new AbortController().signal };
}
function fakeRun(provider, f, action = '', extra = {}) {
  const launch = invocation(provider, f, { ...job, provider });
  return runProvider({ provider, ...launch, executable: process.execPath,
    args: [fake, `--fake-provider=${provider}`, `--fake-action=${action}`, ...launch.args],
    env: providerEnvironment(provider, f.home), prompt: promptFor(job), signal: f.signal, ...extra });
}

test('provider adapters parse real-shaped CLI output, use stdin, and exclude worker credentials', async t => {
  const f = await fixture(t);
  for (const provider of ['codex', 'claude']) {
    const record = join(f.root, `${provider}.json`);
    const result = await fakeRun(provider, f, '', { env: { ...providerEnvironment(provider, f.home, { PATH: process.env.PATH, TAKOMO_LANE_TOKEN: 'must-not-leak', GITHUB_TOKEN: 'must-not-leak' }), FAKE_RECORD: record } });
    assert.equal(result.result, 'Checked the lane.');
    assert.match(result.conversation_ref, new RegExp(`^${provider}:`));
    const received = JSON.parse(await readFile(record, 'utf8'));
    assert.match(received.prompt, /Useful details/);
    assert.equal(received.cwd, f.cwd);
    assert.equal(received.env.TAKOMO_LANE_TOKEN, undefined);
    assert.equal(received.env.GITHUB_TOKEN, undefined);
    assert(!received.args.some(arg => arg.includes('User title')));
  }
});

test('preparation and review enforce read-only tool policies, never resume implementation', () => {
  const f = { cwd: '/tmp/work', home: '/tmp/auth' };
  for (const kind of ['preparation', 'review']) {
    const j = { ...job, kind, target_revision: 'a'.repeat(40) };
    assert(invocation('codex', f, j).args.includes('sandbox_mode="read-only"'));
    const args = invocation('claude', f, j).args;
    assert.equal(args[args.indexOf('--tools') + 1], 'Read,Glob,Grep');
    assert.equal(args[args.indexOf('--permission-mode') + 1], 'plan');
    assert.throws(() => invocation('claude', f, j, '11111111-1111-1111-1111-111111111111'));
  }
  const args = invocation('claude', f, { ...job, kind: 'implementation' }).args;
  const settings = JSON.parse(args[args.indexOf('--settings') + 1]);
  assert.equal(settings.sandbox.failIfUnavailable, true);
  assert.equal(settings.sandbox.allowUnsandboxedCommands, false);
  assert.equal(sessionId('--last', 'codex'), null);
  assert.equal(sessionId('claude:11111111-1111-1111-1111-111111111111', 'codex'), null);
});

test('provider failures and oversized responses never masquerade as completed work', async t => {
  const f = await fixture(t);
  for (const provider of ['codex', 'claude']) {
    for (const action of ['failed', 'invalid', 'exit']) await assert.rejects(fakeRun(provider, f, action), error => !error.message.includes('secret diagnostic'));
  }
  await assert.rejects(fakeRun('codex', f, 'large'), /oversized/);
  await assert.rejects(fakeRun('codex', f, 'hang', { timeoutMs: 30 }), /timed out/);
});

test('cancellation kills a provider and its spawned command', async t => {
  const f = await fixture(t), controller = new AbortController(), pidFile = join(f.root, 'pid');
  const pending = fakeRun('codex', f, 'descendant', { signal: controller.signal,
    env: { ...providerEnvironment('codex', f.home), FAKE_PID: pidFile } });
  const rejected = assert.rejects(pending, /cancelled/);
  let pid;
  for (let i = 0; i < 100; i++) { try { pid = Number(await readFile(pidFile, 'utf8')); break; } catch { await sleep(10); } }
  assert(pid);
  controller.abort(); await rejected;
  // Reaped or zombie means the command cannot execute; Linux may retain a zombie briefly.
  if (process.platform === 'linux') {
    let status;
    try { status = await readFile(`/proc/${pid}/stat`, 'utf8'); } catch (e) { assert.equal(e.code, 'ENOENT'); }
    if (status) assert.equal(status.split(' ')[2], 'Z');
  } else assert.throws(() => process.kill(pid, 0));
});

test('workspace locks serialize workers and read-only reviews require the exact clean revision', async t => {
  const f = await fixture(t);
  const release = await lockWorkspace(f.cwd);
  assert.equal(await lockWorkspace(f.cwd), null);
  await release();
  const again = await lockWorkspace(f.cwd); assert(again); await again();
  const target_revision = await git(f.cwd, ['rev-parse', 'HEAD']);
  const review = { ...job, kind: 'review', target_revision };
  const before = await prepareWorkspace(f.cwd, review);
  assert.deepEqual(await finishWorkspace(f.cwd, review, before), { revision: target_revision });
  await assert.rejects(prepareWorkspace(f.cwd, { ...review, target_revision: 'a'.repeat(40) }), /exact full commit/);
  await writeFile(join(f.cwd, 'untracked.txt'), 'Do not sweep me up');
  await assert.rejects(prepareWorkspace(f.cwd, job), /untracked/);
});

test('implementation produces a real local commit, preserves hooks, and never pushes', async t => {
  const f = await fixture(t), implementation = { ...job, kind: 'implementation' };
  const before = await prepareWorkspace(f.cwd, implementation);
  await writeFile(join(f.cwd, 'change.txt'), 'Implemented');
  const { revision } = await finishWorkspace(f.cwd, implementation, before);
  assert.notEqual(revision, before.revision);
  assert.equal(await git(f.cwd, ['show', `${revision}:change.txt`]), 'Implemented');
  assert.equal(await git(f.cwd, ['remote']), '');
  const next = await prepareWorkspace(f.cwd, implementation);
  const hook = join(f.cwd, '.git/hooks/pre-commit');
  await writeFile(hook, '#!/bin/sh\nexit 1\n'); await chmod(hook, 0o700);
  await writeFile(join(f.cwd, 'change.txt'), 'Unvalidated');
  await assert.rejects(finishWorkspace(f.cwd, implementation, next), /hook failures/);
  assert.equal(await git(f.cwd, ['rev-parse', 'HEAD']), revision);
  assert.match(await git(f.cwd, ['status', '--porcelain']), /change.txt/);
});

test('lease loss stops execution and never delivers stale result', { timeout: 5000 }, async t => {
  const f = await fixture(t); let running = false, stopped = false; const calls = [];
  const api = async (path, body) => {
    calls.push({ path, body });
    if (path.endsWith('/heartbeat') && running) throw new ApiError(409);
    return {};
  };
  await executeHandoff(job, { ...f, api, heartbeatMs: 10, run: ({ signal }) => new Promise((resolve, reject) => {
    running = true;
    signal.addEventListener('abort', () => { stopped = true; reject(new Error('cancelled')); }, { once: true });
  }) });
  assert(stopped);
  assert(!calls.some(call => call.path.endsWith('/result')));
});

test('provider-created commits are rejected so review cannot silently cover only the final commit', async t => {
  const f = await fixture(t), implementation = { ...job, kind: 'implementation' };
  const before = await prepareWorkspace(f.cwd, implementation);
  await writeFile(join(f.cwd, 'agent-change.txt'), 'An agent committed this');
  await git(f.cwd, ['add', '.']);
  await git(f.cwd, ['-c', 'user.name=Test', '-c', 'user.email=test@localhost', '-c', 'commit.gpgsign=false', 'commit', '-m', 'Agent commit']);
  const agentRevision = await git(f.cwd, ['rev-parse', 'HEAD']);
  await assert.rejects(finishWorkspace(f.cwd, implementation, before), /changed HEAD/);
  assert.equal(await git(f.cwd, ['rev-parse', 'HEAD']), agentRevision);
});

test('a no-op implementation gets an empty handoff commit rather than reviewing unrelated history', async t => {
  const f = await fixture(t), implementation = { ...job, kind: 'implementation' };
  const before = await prepareWorkspace(f.cwd, implementation);
  const { revision } = await finishWorkspace(f.cwd, implementation, before);
  assert.notEqual(revision, before.revision);
  assert.equal(await git(f.cwd, ['diff', `${revision}^`, revision]), '');
});

test('completed receipts replay a reclaimed attempt without repeating the model call', async t => {
  const f = await fixture(t); let runs = 0; const delivered = [];
  const api = async (path, body) => { if (path.endsWith('/result')) delivered.push(body); return {}; };
  const run = async () => { runs++; return { result: 'Prepared context' }; };
  await executeHandoff(job, { ...f, api, run });
  await executeHandoff({ ...job, attempt: 2 }, { ...f, api, run });
  assert.equal(runs, 1);
  assert.equal(delivered[1].attempt, 2);
  assert.equal(delivered[1].result, 'Prepared context');
});

test('local provider references resume only the same project/workspace implementation; reviews remain fresh', async t => {
  const f = await fixture(t), launches = [];
  const reference = 'codex:11111111-1111-1111-1111-111111111111';
  const run = async args => { launches.push(args); return { result: 'Done', conversation_ref: reference }; };
  const api = async () => ({});
  const implementation = { ...job, kind: 'implementation', snapshot: { ...job.snapshot, lane: { ...job.snapshot.lane, conversation_ref: reference } } };
  await executeHandoff(implementation, { ...f, api, run });
  assert(!launches[0].args.includes('resume'));
  // The server may still carry a preparation reference; local lane history wins.
  await executeHandoff({ ...implementation, id: 'hf-second', snapshot: job.snapshot }, { ...f, api, run });
  assert(launches[1].args.includes('resume'));
  await executeHandoff({ ...implementation, id: 'hf-review', kind: 'review', target_revision: await git(f.cwd, ['rev-parse', 'HEAD']) }, { ...f, api, run });
  assert(!launches[2].args.includes('resume'));
  assert.match(launches[2].prompt, /EXACT COMMIT REVIEW EVIDENCE/);
  assert.match(launches[2].prompt, /Implement Takomo handoff hf-second/);
  assert.doesNotMatch(launches[2].prompt, /\+Initial/);
  assert.match(await reviewEvidence(f.cwd, await git(f.cwd, ['rev-parse', 'HEAD']), 10), /DIFF TRUNCATED/);
});

test('queue scans beyond unsupported providers and never dispatches work itself', async t => {
  const f = await fixture(t), paths = [];
  delete f.config.providers.claude;
  const api = async (path, body) => {
    paths.push(path);
    if (path.includes('offset=0')) return { items: Array.from({ length: 100 }, (_, i) => ({ id: `other-${i}`, provider: 'claude' })), total: 101 };
    if (path.includes('offset=100')) return { items: [job], total: 101 };
    if (path.endsWith('/claim')) return job;
    return {};
  };
  assert.equal(await runNext({ ...f, api, run: async () => ({ result: 'Prepared' }) }), true);
  assert(paths.some(path => path.includes('offset=100')));
  assert(paths.filter(path => path.endsWith('/claim')).every(path => path.includes(job.id)));
  assert(!paths.some(path => path.endsWith('/handoffs')));
});

test('configuration rejects ambiguous workspaces and credential homes inside the checkout', async t => {
  const f = await fixture(t), file = join(f.root, 'config.json');
  await writeFile(file, JSON.stringify({ projects: { one: f.cwd, two: f.cwd }, providers: { codex: { home: f.home } } }));
  await assert.rejects(loadConfig(file), /must not share/);
  await writeFile(file, JSON.stringify({ projects: { one: f.cwd }, providers: { codex: { home: join(f.cwd, 'auth') } } }));
  await assert.rejects(loadConfig(file), /outside project/);
  assert.throws(() => apiClient('http://example.com', 'secret', f.signal), /HTTPS/);
  assert.throws(() => apiClient('https://user:pass@example.com', 'secret', f.signal), /without credentials/);
});

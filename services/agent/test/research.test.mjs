import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { Codex, configArgs, profileFor, researchRestrictions, restrictions, validateConfig } from '../codex.mjs';
import { executeJob } from '../service.mjs';
import { openRepository } from '../repository.mjs';

async function fixture(t) {
  const cwd = await mkdtemp(join(tmpdir(), 'takomo-research-'));
  t.after(() => rm(cwd, { recursive: true, force: true }));
  const git = args => execFileSync('git', args, { cwd, stdio: 'pipe' }).toString().trim();
  git(['init']);
  await writeFile(join(cwd, 'sample.js'), 'export const broken = true;\n');
  await symlink('/etc/passwd', join(cwd, 'outside'));
  git(['add', '.']);
  git(['-c', 'user.name=Test', '-c', 'user.email=test@example.com', '-c', 'core.hooksPath=/dev/null', 'commit', '-m', 'fixture']);
  const revision = git(['rev-parse', 'HEAD']);
  await writeFile(join(cwd, 'sample.js'), 'uncommitted content must not be visible');
  const job = { id: 'job', attempt_id: 'attempt', kind: 'bug_research', project: 'demo', repository_ref: { repository: 'demo', revision: 'HEAD' }, prompt: 'Inspect', snapshot: '{"title":"Broken"}' };
  const repositories = { demo: cwd };
  const args = ['app-server', '--stdio', ...configArgs(profileFor('bug_research'))];
  const createCodex = () => new Codex({ executable: process.execPath, args: [fileURLToPath(new URL('./fake-research.mjs', import.meta.url)), ...args], cwd: '/tmp', home: '/tmp', repositories, timeoutMs: 500, kind: 'bug_research' });
  return { job, revision, repositories, createCodex };
}
test('research reads committed source and records exact revision evidence via app server', async t => {
  const { job, revision, createCodex } = await fixture(t);
  const codex = createCodex();
  try {
    const result = await codex.run(job);
    assert.match(result.message, /broken = true/);
    assert.doesNotMatch(result.message, /uncommitted/);
    assert.equal(result.repository_revision, revision);
    assert.equal(result.evidence.inspected[0].path, 'sample.js');
    assert.equal(result.evidence.runtime_reproduced, false);
  } finally { codex.close(); }
});
test('repository allowlist and file inventory reject arbitrary paths and symlinks', async t => {
  const { job, repositories } = await fixture(t);
  await assert.rejects(openRepository({ ...job, repository_ref: { repository: '/etc' } }, repositories), /not configured/);
  const repo = await openRepository(job, repositories);
  await assert.rejects(repo.call('repository_read', { path: 'outside' }), /regular tracked/);
  await assert.rejects(repo.call('repository_read', { path: '../../etc/passwd' }), /regular tracked/);
  assert.equal(JSON.parse(await repo.call('repository_files', {})).total, 1);
  assert.match(JSON.parse(await repo.call('repository_search', { query: 'broken' })).matches[0], /sample.js:1/);
  assert.equal(JSON.parse(await repo.call('repository_search', { query: 'uncommitted' })).total, 0);
});
test('heartbeat steering reaches the active turn, cancellation interrupts, timeout does not rerun', async t => {
  const { job, createCodex } = await fixture(t);
  for (const mode of ['STEER', 'CANCEL', 'TIMEOUT']) {
    let runs = 0;
    let result;
    await executeJob({ ...job, prompt: mode === 'STEER' ? mode : 'HANG' }, {
      createCodex: () => { runs++; return createCodex(); },
      serviceId: 'worker', signal: new AbortController().signal, heartbeatMs: 20,
      api: async (path, body) => {
        if (path.endsWith('/result')) result = body;
        if (body.turn_id && mode === 'CANCEL') return { cancel_requested: true };
        if (mode === 'STEER') return { steering: [{ id: 'steering-1', message: 'Check dates' }] };
        return {};
      },
    });
    assert.equal(runs, 1);
    assert.equal(result.status, mode === 'STEER' ? 'completed' : 'failed');
    if (mode === 'STEER') assert.match(result.message, /Check dates/);
    if (mode === 'CANCEL') assert.equal(result.cancelled, true);
    if (mode === 'TIMEOUT') assert.match(result.error, /timed out/);
  }
});

test('revision is heartbeated before session start and remains in timeout evidence', async t => {
  const { job, revision, createCodex } = await fixture(t);
  const heartbeats = [];
  let result;
  await executeJob({ ...job, prompt: 'HANG' }, {
    createCodex, serviceId: 'worker', signal: new AbortController().signal,
    api: async (path, body) => {
      if (path.endsWith('/heartbeat')) heartbeats.push(body);
      if (path.endsWith('/result')) result = body;
      return {};
    },
  });
  assert.equal(heartbeats[0].repository_revision, revision);
  assert.equal(heartbeats[0].thread_id, undefined);
  assert.equal(result.repository_revision, revision);
  assert.equal(result.status, 'failed');
  // Recovery is a separate, explicitly submitted attempt, not a replay of the failed turn.
  const next = createCodex();
  try { assert.equal((await next.run({ ...job, id: 'explicit-retry' })).repository_revision, revision); }
  finally { next.close(); }
});

test('missing repository or app server produces one failed job without automatic execution retry', async t => {
  const { job, repositories } = await fixture(t);
  for (const missing of ['repository', 'server']) {
    let runs = 0;
    let result;
    await executeJob(job, {
      serviceId: 'worker', signal: new AbortController().signal,
      createCodex: () => {
        runs++;
        return new Codex({ executable: missing === 'server' ? '/nonexistent/takomo-codex' : process.execPath,
          args: [fileURLToPath(new URL('./fake-research.mjs', import.meta.url))], cwd: '/tmp', home: '/tmp', repositories: missing === 'repository' ? {} : repositories, kind: 'bug_research' });
      },
      api: async (path, body) => { if (path.endsWith('/result')) result = body; return {}; },
    });
    assert.equal(runs, 1);
    assert.equal(result.status, 'failed');
    assert.match(result.error, missing === 'repository' ? /not configured/ : /ENOENT/);
  }
});


test('cancellation preserves already inspected evidence in heartbeats and failed result', async t => {
  const { job, revision, createCodex } = await fixture(t);
  let result;
  let partial;
  await executeJob({ ...job, prompt: 'READ_HANG' }, {
    createCodex, serviceId: 'worker', signal: new AbortController().signal, heartbeatMs: 10,
    api: async (path, body) => {
      if (path.endsWith('/heartbeat') && body.evidence?.inspected.length) {
        partial = body;
        return { cancel_requested: true };
      }
      if (path.endsWith('/result')) result = body;
      return {};
    },
  });
  assert.equal(partial.repository_revision, revision);
  assert.equal(result.cancelled, true);
  assert.equal(result.status, 'failed');
  assert.equal(result.evidence.inspected[0].path, 'sample.js');
  assert.equal(result.message, undefined);
});

test('research enables only the dynamic-tool host; section review keeps every feature disabled', async t => {
  const { job, repositories, createCodex } = await fixture(t);
  const differing = Object.keys(restrictions.features).filter(name => restrictions.features[name] !== researchRestrictions.features[name]);
  assert.deepEqual(differing, ['code_mode_host']);
  assert.equal(researchRestrictions.features.code_mode, false);
  assert.equal(researchRestrictions.features.shell_tool, false);
  assert.equal(researchRestrictions.sandbox_mode, 'read-only');
  assert.equal(profileFor(undefined), restrictions);
  assert.equal(profileFor('section_review'), restrictions);
  validateConfig(researchRestrictions, researchRestrictions);
  // The effective configuration the live smoke saw: host disabled, so every repository tool call was refused.
  assert.throws(() => validateConfig(restrictions, researchRestrictions), /enable code_mode_host/);
  assert.throws(() => validateConfig(researchRestrictions), /disable code_mode_host/);
  // A process spawned with the section profile is refused a research job before any tool is offered, and the reverse.
  const section = new Codex({ executable: process.execPath, args: [fileURLToPath(new URL('./fake-research.mjs', import.meta.url))], cwd: '/tmp', home: '/tmp', repositories });
  try { await assert.rejects(section.run(job), /started for section review/); } finally { section.close(); }
  const research = createCodex();
  try { await assert.rejects(research.run({ ...job, kind: undefined }), /started for research/); } finally { research.close(); }
  // Startup flags, thread config and the effective-config check must all agree, or the fake exits at thread/start.
  const stale = new Codex({ executable: process.execPath, args: [fileURLToPath(new URL('./fake-research.mjs', import.meta.url)), ...configArgs(restrictions)], cwd: '/tmp', home: '/tmp', repositories, timeoutMs: 500, kind: 'bug_research' });
  try { await assert.rejects(stale.run(job), /enable code_mode_host/); } finally { stale.close(); }
});

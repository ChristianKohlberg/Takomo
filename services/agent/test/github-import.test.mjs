import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { executeGithubImport, prepareGithubSource } from '../github-import.mjs';

const job = { id: 'ci-test', attempt_id: 'attempt', source: { revision: 'a'.repeat(40), full_name: 'owner/repo', scope: { include: ['src'], exclude: [] }, limits: { max_files: 20, max_source_bytes: 100000, max_tool_calls: 12 }, max_sections: 3 } };
test('GitHub extraction keeps credentials out of inference and retries only result delivery', async () => {
 const state = await mkdtemp(join(tmpdir(), 'github-import-test-'));
 const calls = []; let generations = 0; let results = 0;
 try {
  await executeGithubImport(job, { state, serviceId: 'worker', signal: new AbortController().signal,
   api: async (path, body) => { calls.push({ path, body }); if (path.endsWith('/source-token')) return { token: 'private-installation-token' }; if (path.endsWith('/result') && ++results === 1) throw new Error('Network interrupted'); return {}; },
   prepare: async (source, directory, token) => { assert.equal(source, job.source); assert.equal(token, 'private-installation-token'); assert.ok(directory.startsWith(state)); },
   generate: async options => { generations++; assert.equal(options.revision, job.source.revision); assert.deepEqual(options.scope, job.source.scope); assert.equal(options.maxSections, 3); assert.ok(!JSON.stringify(options).includes('private-installation-token')); return { request: { draft: { title: 'Draft' } } }; },
  });
  assert.equal(generations, 1); assert.equal(results, 2); assert.ok(!JSON.stringify(calls).includes('private-installation-token'));
  assert.deepEqual(await readdir(state), []);
 } finally { await rm(state, { recursive: true, force: true }); }
});
test('source preparation failure does not start a model and is reported once', async () => {
 const state = await mkdtemp(join(tmpdir(), 'github-import-failed-')); const results = [];
 try {
  await executeGithubImport(job, { state, serviceId: 'worker', signal: new AbortController().signal, api: async (p, body) => { if (p.endsWith('/source-token')) return { token: 'secret' }; results.push(body); return {}; }, prepare: async () => { throw new Error('Selected source exceeds the byte budget.'); }, generate: () => assert.fail('No inference before source preflight') });
  assert.equal(results.length, 1); assert.match(results[0].error, /byte budget/); assert.deepEqual(await readdir(state), []);
 } finally { await rm(state, { recursive: true, force: true }); }
});
test('invalid repository/scope is rejected before network or Git commands', async () => {
 await assert.rejects(prepareGithubSource({ ...job.source, full_name: 'https://attacker.invalid/repo' }, '/does/not/exist', 'secret', new AbortController().signal), /Invalid pinned/);
 await assert.rejects(prepareGithubSource({ ...job.source, scope: { include: ['../private'] } }, '/does/not/exist', 'secret', new AbortController().signal), /relative/);
});

test('hydrate retrieves only selected blobs, verifies hashes and supports excluded missing objects', async () => {
 const { execFileSync } = await import('node:child_process');
 const { mkdir, writeFile, readFile, unlink } = await import('node:fs/promises');
 const { hydrateGithubSource } = await import('../github-import.mjs');
 const { openRepository } = await import('../repository.mjs');
 const directory = await mkdtemp(join(tmpdir(), 'github-hydrate-'));
 const command = (...args) => execFileSync('git', args, { cwd: directory, encoding: 'utf8', env: { ...process.env, GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null' }, stdio: ['ignore', 'pipe', 'ignore'] }).trim();
 try {
  command('init'); await mkdir(join(directory, 'src'));
  await writeFile(join(directory, 'src/sample.js'), 'export const count = 1;\n');
  await writeFile(join(directory, 'src/excluded.js'), 'private sample');
  command('add', '.'); command('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-m', 'fixture');
  const revision = command('rev-parse', 'HEAD');
  const selected = command('rev-parse', 'HEAD:src/sample.js');
  const excluded = command('rev-parse', 'HEAD:src/excluded.js');
  const content = await readFile(join(directory, 'src/sample.js'));
  for (const oid of [selected, excluded]) await unlink(join(directory, '.git/objects', oid.slice(0, 2), oid.slice(2)));
  const source = { ...job.source, revision, scope: { include: ['src'], exclude: ['src/excluded.js'] } };
  const urls = [];
  await hydrateGithubSource(source, directory, 'secret', new AbortController().signal, async url => {
   urls.push(url); return new Response(JSON.stringify({ encoding: 'base64', content: content.toString('base64') }));
  });
  assert.deepEqual(urls, [`https://api.github.com/repos/owner/repo/git/blobs/${selected}`]);
  const repo = await openRepository({ repository_ref: { repository: 'test', revision, scope: source.scope } }, { test: directory }, source.limits);
  assert.equal(repo.revision, revision); assert.equal(repo.manifest().counts.selected_files, 1);
  await assert.rejects(hydrateGithubSource(source, directory, 'secret', new AbortController().signal, async () => new Response(JSON.stringify({ encoding: 'base64', content: Buffer.from('tampered').toString('base64') }))), /did not match/);
 } finally { await rm(directory, { recursive: true, force: true }); }
});

import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { repositoryScope, underPath } from './repository-scope.mjs';
import { repositoryGit } from './repository-git.mjs';
import { generateImport } from './spec-import-cli.mjs';

// Credentials are environment-only, never URLs, command arguments, artifacts or model input.
function git(cwd, args, { token, input, signal } = {}) {
  return new Promise((resolve, reject) => {
    const env = { PATH: process.env.PATH, GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null', GIT_TERMINAL_PROMPT: '0', GIT_NO_LAZY_FETCH: '1', GIT_CONFIG_COUNT: token ? '1' : '0' };
    if (token) { env.GIT_CONFIG_KEY_0 = 'http.https://github.com/.extraheader'; env.GIT_CONFIG_VALUE_0 = `Authorization: Basic ${Buffer.from(`x-access-token:${token}`).toString('base64')}`; }
    const child = spawn('git', ['--no-replace-objects', '-c', 'core.hooksPath=/dev/null', '-c', 'protocol.file.allow=never', '-c', 'http.followRedirects=false', ...args], { cwd, env, stdio: ['pipe', 'pipe', 'pipe'], signal });
    let output = ''; let failed = false;
    const timer = setTimeout(() => { failed = true; child.kill(); }, 60_000);
    child.stdout.on('data', chunk => { output += chunk; if (output.length > 1_000_000) { failed = true; child.kill(); } });
    child.stderr.resume(); child.stdin.on('error', () => {}); child.stdin.end(input);
    child.on('error', () => { failed = true; });
    child.on('close', code => { clearTimeout(timer); if (failed || code !== 0) reject(new Error('Git source preparation failed or exceeded its time/output limit. Check repository access and retry explicitly.')); else resolve(output); });
  });
}
export async function prepareGithubSource(source, directory, token, signal, fetcher = fetch) {
  repositoryScope(source.scope);
  if (!/^[\w.-]+\/[\w.-]+$/.test(source.full_name) || !/^[a-f0-9]{40}$/.test(source.revision)) throw new Error('Invalid pinned GitHub source.');
  await git(directory, ['init', '--bare', '.'], { signal });
  // Fetch commit/tree metadata, without source blobs or a working checkout.
  // Metadata still depends on repository size and has a one-minute deadline.
  await git(directory, ['fetch', '--no-tags', '--depth=1', '--filter=blob:none', `https://github.com/${source.full_name}.git`, source.revision], { token, signal });
  return hydrateGithubSource(source, directory, token, signal, fetcher);
}
export async function hydrateGithubSource(source, directory, token, signal, fetcher = fetch) {
  const scope = repositoryScope(source.scope);
  const reader = repositoryGit(directory);
  const listing = await reader(['ls-tree', '-rz', '--full-tree', source.revision, '--', ...scope.include.filter(p => p !== '.')], { maxBytes: 1_000_000 });
  const files = listing.split('\0').filter(Boolean).map(line => {
    const match = /^(\d+) (\w+) ([a-f0-9]+)\t([\s\S]+)$/.exec(line);
    if (!match) throw new Error('Invalid repository tree.');
    return { mode: match[1], type: match[2], oid: match[3], path: match[4] };
  }).filter(f => ['100644', '100755'].includes(f.mode) && f.type === 'blob' && !scope.exclude.some(p => underPath(f.path, p)));
  if (!files.length || files.length > source.limits.max_files) throw new Error('Selected source exceeds the file budget or is empty. Choose a smaller, existing path.');
  let total = 0;
  for (const file of files) {
    const response = await fetcher(`https://api.github.com/repos/${source.full_name}/git/blobs/${file.oid}`, { headers: { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28', 'User-Agent': 'takomo-agent/0.1' }, redirect: 'error', signal: AbortSignal.any([signal, AbortSignal.timeout(20_000)]) });
    if (!response.ok) throw new Error('GitHub source access failed. Refresh the connection before retrying.');
    const chunks = []; let bytes = 0;
    for await (const chunk of response.body) { bytes += chunk.length; if (bytes > 150_000) throw new Error('A selected source blob exceeds the development budget.'); chunks.push(chunk); }
    const blob = JSON.parse(Buffer.concat(chunks).toString('utf8'));
    if (blob.encoding !== 'base64' || typeof blob.content !== 'string') throw new Error('Unsupported GitHub source encoding.');
    const content = Buffer.from(blob.content, 'base64'); total += content.length;
    if (total > source.limits.max_source_bytes) throw new Error('Selected source exceeds the byte budget. Choose a smaller path.');
    const oid = (await git(directory, ['hash-object', '-w', '--stdin'], { input: content, signal })).trim();
    if (oid !== file.oid) throw new Error('GitHub source did not match the pinned Git object.');
  }
  return directory;
}
export async function executeGithubImport(job, { api, serviceId, state, signal, prepare = prepareGithubSource, generate = generateImport }) {
  const identity = { service_id: serviceId, attempt_id: job.attempt_id };
  const prefix = `/v1/codebase-import-jobs/${encodeURIComponent(job.id)}`;
  const controller = new AbortController();
  const combined = AbortSignal.any([signal, controller.signal]);
  let heartbeatChain = Promise.resolve();
  const timer = setInterval(() => { heartbeatChain = heartbeatChain.then(() => api(`${prefix}/heartbeat`, identity)).catch(() => { controller.abort(); }); }, 15_000);
  let directory;
  try {
    const { token } = await api(`${prefix}/source-token`, identity);
    directory = await mkdtemp(join(state, 'github-import-'));
    await prepare(job.source, directory, token, combined);
    if (combined.aborted) return;
    // The generic CLI owns the bounded App Server turn and recoverable artifact.
    const artifact = await generate({ repository: directory, revision: job.source.revision, scope: job.source.scope, limits: job.source.limits, maxSections: job.source.max_sections, output: join(state, `${job.id}.json`), stateDir: state, signal: combined });
    if (combined.aborted) return;
    await deliver({ ...identity, draft: artifact.request.draft });
  } catch (error) {
    if (!combined.aborted) await deliver({ ...identity, error: error.message.slice(0, 2000) });
  } finally { clearInterval(timer); await heartbeatChain; if (directory) await rm(directory, { recursive: true, force: true }); }
  async function deliver(body) {
    for (let attempt = 0; attempt < 3; attempt++) {
      if (combined.aborted) return;
      try { await api(`${prefix}/result`, body); return; } catch (error) { if (attempt === 2 || error.status && error.status < 500 && error.status !== 429) throw error; }
    }
  }
}

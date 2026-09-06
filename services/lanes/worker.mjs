import { mkdir, readFile, writeFile, realpath, rm, rename } from 'node:fs/promises';
import { resolve, join, isAbsolute } from 'node:path';
import { homedir } from 'node:os';
import { createHash, randomUUID } from 'node:crypto';
import { spawn } from 'node:child_process';
import { pathToFileURL } from 'node:url';
import { setTimeout as sleep } from 'node:timers/promises';
import { invocation, promptFor, providerEnvironment, runProvider, sessionId } from './providers.mjs';

const sha = value => createHash('sha256').update(value).digest('hex');
export class ApiError extends Error {
  constructor(status) { super(`Takomo returned HTTP ${status}.`); this.status = status; }
}
export function apiClient(url, token, signal) {
  const base = new URL(url);
  if (base.username || base.password || base.search || base.hash || !['', '/'].includes(base.pathname)) throw new Error('TAKOMO_URL must be an origin without credentials, path, query or fragment.');
  if (base.protocol !== 'https:' && !(base.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(base.hostname))) throw new Error('Use HTTPS for remote Takomo; HTTP is allowed only on loopback.');
  return async (path, body) => {
    const response = await fetch(`${base.origin}${path}`, {
      method: body === undefined ? 'GET' : 'POST', redirect: 'error',
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      signal: AbortSignal.any([signal, AbortSignal.timeout(10_000)]),
    });
    if (!response.ok) throw new ApiError(response.status);
    return response.json();
  };
}

export async function loadConfig(file) {
  const config = JSON.parse(await readFile(file, 'utf8'));
  if (!config.projects || !Object.keys(config.projects).length || !config.providers || !Object.keys(config.providers).length) throw new Error('Configure explicit projects and providers in the worker config.');
  for (const [project, path] of Object.entries(config.projects)) {
    if (!project || typeof path !== 'string' || !isAbsolute(path)) throw new Error('Every project must map to an absolute dedicated Git checkout.');
    config.projects[project] = await realpath(path);
  }
  if (new Set(Object.values(config.projects)).size !== Object.keys(config.projects).length) throw new Error('Projects must not share a worker checkout.');
  for (const [name, provider] of Object.entries(config.providers)) {
    if (!['codex', 'claude'].includes(name) || !provider || typeof provider.home !== 'string' || !isAbsolute(provider.home)) throw new Error('Each provider needs its own absolute home; supported providers are codex and claude.');
    if (provider.model !== undefined && (typeof provider.model !== 'string' || !provider.model)) throw new Error('Provider model must be a nonempty string.');
    if (provider.executable !== undefined && typeof provider.executable !== 'string') throw new Error('Provider executable must be a local command or path, not an argument array.');
    await mkdir(provider.home, { recursive: true, mode: 0o700 });
    provider.home = await realpath(provider.home);
  }
  const homes = Object.values(config.providers).map(p => p.home);
  if (new Set(homes).size !== homes.length) throw new Error('Providers need separate credential homes.');
  for (const cwd of Object.values(config.projects)) {
    for (const home of homes) if (home === cwd || home.startsWith(`${cwd}/`) || cwd.startsWith(`${home}/`)) throw new Error('Provider credential homes must be outside project workspaces.');
  }
  return config;
}

/** No worker/server secrets reach Git hooks or agent-launched commands. */
export async function git(cwd, args, signal = new AbortController().signal) {
  return new Promise((resolve, reject) => {
    if (signal.aborted) { reject(new Error('Local Git operation cancelled.')); return; }
    const child = spawn('git', args, {
      cwd, env: { PATH: process.env.PATH, HOME: homedir(), LANG: 'C.UTF-8', GIT_TERMINAL_PROMPT: '0' },
      detached: true, shell: false, stdio: ['ignore', 'pipe', 'ignore'],
    });
    let output = '', failed = false;
    const kill = () => { if (child.pid) { try { process.kill(-child.pid, 'SIGKILL'); } catch {} } };
    const stop = () => { failed = true; kill(); };
    signal.addEventListener('abort', stop, { once: true });
    const timer = setTimeout(stop, 120_000);
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', chunk => { output += chunk; if (Buffer.byteLength(output) > 4_000_000) stop(); });
    child.on('error', () => { failed = true; });
    child.on('exit', kill);
    child.on('close', code => {
      clearTimeout(timer); signal.removeEventListener('abort', stop);
      if (failed || code !== 0) reject(new Error('Local Git operation failed or was cancelled. Inspect the dedicated checkout and any project hook failures before retrying.'));
      else resolve(output.trim());
    });
  });
}

export async function lockWorkspace(cwd) {
  const root = await realpath(await git(cwd, ['rev-parse', '--show-toplevel']));
  if (root !== cwd) throw new Error('Workspace mapping must name the checkout root.');
  const lock = resolve(cwd, await git(cwd, ['rev-parse', '--git-path', 'takomo-lanes.lock']));
  try { await mkdir(lock, { mode: 0o700 }); }
  catch (error) { if (error.code === 'EEXIST') return null; throw error; }
  try { await writeFile(join(lock, 'owner.json'), JSON.stringify({ pid: process.pid, started_at: new Date().toISOString() }), { mode: 0o600 }); }
  catch (error) { await rm(lock, { recursive: true }); throw error; }
  // Do not automatically steal a stale lock: a child from a crashed worker may still run.
  return () => rm(lock, { recursive: true });
}

async function workspaceState(cwd) {
  return {
    revision: await git(cwd, ['rev-parse', '--verify', 'HEAD']),
    dirty: !!await git(cwd, ['status', '--porcelain', '--untracked-files=all']),
  };
}

export async function prepareWorkspace(cwd, job) {
  const before = await workspaceState(cwd);
  if (before.dirty) throw new Error('The dedicated checkout has uncommitted or untracked changes. Preserve and resolve them locally before retrying.');
  if (job.kind === 'review' && (!/^[0-9a-f]{40,64}$/i.test(job.target_revision ?? '') || before.revision !== job.target_revision)) throw new Error('Review target must be the exact full commit currently checked out in a clean mapped workspace.');
  if (job.kind === 'implementation') {
    before.branch = await git(cwd, ['symbolic-ref', '--quiet', '--short', 'HEAD']);
    if (['main', 'master'].includes(before.branch)) throw new Error('Implementation requires a dedicated local work branch, not main or master.');
  }
  return before;
}

export async function finishWorkspace(cwd, job, before, signal) {
  const after = await workspaceState(cwd);
  if (job.kind !== 'implementation') {
    if (after.dirty || before.revision !== after.revision) throw new Error('The read-only workspace changed during execution. Result was not accepted; inspect the checkout.');
    return job.kind === 'review' ? { revision: before.revision } : {};
  }
  if (before.branch !== await git(cwd, ['symbolic-ref', '--quiet', '--short', 'HEAD'])) throw new Error('Implementation changed branches. Inspect the checkout before retrying.');
  if (after.revision !== before.revision) throw new Error('Implementation changed HEAD. The worker must create the sole handoff commit; inspect the checkout before retrying.');
  if (after.dirty) {
    await git(cwd, ['add', '--all', '--', '.'], signal);
  }
  // Even a no-op gets its own revision, so review never inspects an unrelated prior commit.
  // Hooks remain enabled: a failed validation leaves a recoverable checkout.
  await git(cwd, ['-c', 'commit.gpgsign=false', '-c', 'user.name=Takomo lane worker', '-c', 'user.email=lanes@localhost', 'commit', '--allow-empty', '-m', `Implement Takomo handoff ${job.id}`], signal);
  const final = await workspaceState(cwd);
  if (final.dirty) throw new Error('Implementation or a commit hook left pending changes. Inspect the checkout before retrying.');
  return { revision: final.revision };
}

async function readJson(file) {
  try { return JSON.parse(await readFile(file, 'utf8')); }
  catch (error) { if (error.code === 'ENOENT') return null; throw error; }
}
async function saveJson(file, data) {
  const temp = `${file}.${randomUUID()}.tmp`;
  await writeFile(temp, JSON.stringify(data), { mode: 0o600, flag: 'wx' });
  await rename(temp, file);
}
const referenceFile = (state, project, cwd, reference) => join(state, `session-${sha(JSON.stringify([project, cwd, reference]))}.json`);
const laneSessionFile = (state, job, cwd) => join(state, `lane-session-${sha(JSON.stringify([job.project, job.lane, cwd, job.provider]))}.json`);

export async function reviewEvidence(cwd, revision, limit = 120_000) {
  const patch = await git(cwd, ['show', '--format=fuller', '--stat', '--patch', '--no-ext-diff', '--no-textconv', '--no-renames', revision, '--']);
  const bytes = Buffer.from(patch);
  return `\n\nEXACT COMMIT REVIEW EVIDENCE (git show ${revision}; compare this commit with its parent):\n${bytes.subarray(0, limit).toString('utf8')}${bytes.length > limit ? '\n[DIFF TRUNCATED: inspect the relevant source files; explicitly report coverage limits.]' : ''}\n`;
}

export async function executeHandoff(job, { api, config, state, signal, heartbeatMs = 20_000, timeoutMs = 1_800_000, run = runProvider }) {
  const cwd = config.projects[job.project];
  const provider = config.providers[job.provider];
  if (!cwd || !provider || !['preparation', 'implementation', 'review'].includes(job.kind)) throw new Error('Claimed handoff is outside the configured worker scope.');
  const path = `/v1/handoffs/${encodeURIComponent(job.id)}`;
  const controller = new AbortController();
  const stop = () => controller.abort();
  signal.addEventListener('abort', stop, { once: true });
  if (signal.aborted) stop();
  let lost = false, chain = Promise.resolve();
  const heartbeat = () => {
    chain = chain.then(async () => {
      if (lost || signal.aborted) throw new Error('Handoff lease lost.');
      try { await api(`${path}/heartbeat`, { attempt: job.attempt }); }
      catch (error) { lost = true; stop(); throw error; }
    });
    return chain;
  };
  const interval = setInterval(() => heartbeat().catch(() => {}), heartbeatMs);
  const receiptFile = join(state, `handoff-${sha(job.id)}.json`);
  try {
    await heartbeat();
    let receipt = await readJson(receiptFile);
    if (receipt && (receipt.project !== job.project || receipt.provider !== job.provider)) throw new Error('Local handoff receipt scope differs from the claimed job.');
    let result = receipt?.result;
    if (!result) {
      try {
        const before = await prepareWorkspace(cwd, job);
        const localLane = job.kind === 'implementation' && await readJson(laneSessionFile(state, job, cwd));
        const reference = localLane?.conversation_ref ?? job.snapshot?.lane?.conversation_ref;
        const candidate = job.kind === 'implementation' ? sessionId(reference, job.provider) : null;
        const known = candidate && await readJson(referenceFile(state, job.project, cwd, reference));
        const launch = invocation(job.provider, { ...provider, cwd }, job, known ? candidate : null);
        const evidence = job.kind === 'review' ? await reviewEvidence(cwd, before.revision) : '';
        if (controller.signal.aborted) throw new Error('Handoff cancelled before provider execution.');
        const response = await run({ provider: job.provider, executable: provider.executable ?? job.provider,
          ...launch, env: providerEnvironment(job.provider, provider.home), prompt: promptFor(job) + evidence, signal: controller.signal, timeoutMs });
        // Confirm the fence before making the implementation's local commit.
        await heartbeat();
        const revision = await finishWorkspace(cwd, job, before, controller.signal);
        result = { status: 'completed', ...response, ...revision };
        if (response.conversation_ref && job.kind === 'implementation') {
          await saveJson(referenceFile(state, job.project, cwd, response.conversation_ref), { provider: job.provider });
          await saveJson(laneSessionFile(state, job, cwd), { conversation_ref: response.conversation_ref });
        }
      } catch (error) { result = { status: 'failed', result: error.message.slice(0, 2000) }; }
      if ((lost || signal.aborted) && result.status !== 'completed') return;
      // Persist before delivery: a reclaimed lease reports this outcome, never reruns a completed model turn.
      await saveJson(receiptFile, { project: job.project, provider: job.provider, result });
    }
    if (lost || signal.aborted) return;
    for (let retry = 0; retry < 5; retry++) {
      await heartbeat();
      try { await api(`${path}/result`, { attempt: job.attempt, ...result }); return result; }
      catch (error) {
        if (lost || signal.aborted || error instanceof ApiError && error.status < 500 && error.status !== 429 || retry === 4) throw error;
        await sleep(Math.min(500 * 2 ** retry, 4_000), undefined, { signal });
      }
    }
  } finally {
    clearInterval(interval);
    signal.removeEventListener('abort', stop);
    controller.abort();
    await chain.catch(() => {});
  }
}

export async function runNext({ api, config, state, signal, ...execution }) {
  for (const [project, cwd] of Object.entries(config.projects)) {
    if (signal.aborted) return false;
    const release = await lockWorkspace(cwd);
    if (!release) continue;
    try {
      for (let offset = 0; ; offset += 100) {
        const page = await api(`/v1/projects/${encodeURIComponent(project)}/handoffs?status=ready&limit=100&offset=${offset}`);
        for (const entry of page.items) {
          if (!config.providers[entry.provider]) continue;
          let job;
          try { job = await api(`/v1/handoffs/${encodeURIComponent(entry.id)}/claim`, {}); }
          catch (error) { if (error instanceof ApiError && [404, 409].includes(error.status)) continue; throw error; }
          if (job.project !== project) throw new Error('Server returned a handoff from another project.');
          await executeHandoff(job, { api, config, state, signal, ...execution });
          return true;
        }
        if (!page.items.length || offset + page.items.length >= page.total || signal.aborted) break;
      }
    } finally { await release(); }
  }
  return false;
}

export async function main() {
  if (process.platform === 'win32') throw new Error('The lane worker requires POSIX process groups (Linux, macOS or WSL2).');
  const env = process.env;
  if (!env.TAKOMO_URL || !env.TAKOMO_LANE_TOKEN || !env.TAKOMO_LANE_CONFIG) throw new Error('Set TAKOMO_URL, TAKOMO_LANE_TOKEN and TAKOMO_LANE_CONFIG.');
  const config = await loadConfig(env.TAKOMO_LANE_CONFIG);
  const configuredState = resolve(env.TAKOMO_LANE_STATE_DIR ?? join(homedir(), '.takomo-lanes'));
  await mkdir(configuredState, { recursive: true, mode: 0o700 });
  const state = await realpath(configuredState);
  for (const cwd of Object.values(config.projects)) if (state === cwd || state.startsWith(`${cwd}/`)) throw new Error('Worker state must be outside mapped workspaces.');
  const timeoutMs = Number(env.TAKOMO_LANE_TIMEOUT_MS ?? 1_800_000);
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1000 || timeoutMs > 86_400_000) throw new Error('TAKOMO_LANE_TIMEOUT_MS must be between 1000 and 86400000.');
  const controller = new AbortController();
  const stop = () => controller.abort();
  process.once('SIGINT', stop); process.once('SIGTERM', stop);
  const api = apiClient(env.TAKOMO_URL, env.TAKOMO_LANE_TOKEN, controller.signal);
  try {
    do {
      try {
        const worked = await runNext({ api, config, state, signal: controller.signal, timeoutMs });
        if (process.argv.includes('--once')) return;
        if (!worked) await sleep(3000, undefined, { signal: controller.signal });
      } catch (error) {
        if (controller.signal.aborted) return;
        if (error instanceof ApiError && [401, 403].includes(error.status) || process.argv.includes('--once')) throw error;
        console.error(error.message);
        await sleep(3000, undefined, { signal: controller.signal });
      }
    } while (!controller.signal.aborted);
  } finally { process.removeListener('SIGINT', stop); process.removeListener('SIGTERM', stop); }
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch(error => { if (error.name !== 'AbortError') { console.error(error.message); process.exitCode = 1; } });
}

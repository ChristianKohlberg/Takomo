import { mkdir, readFile, writeFile, readdir } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { homedir } from 'node:os';
import { randomUUID } from 'node:crypto';
import { pathToFileURL } from 'node:url';
import { setTimeout as sleep } from 'node:timers/promises';
import { Codex } from './codex.mjs';

export class ApiError extends Error {
  constructor(status) { super(`Takomo returned HTTP ${status}.`); this.status = status; }
}
export function apiClient(url, token, signal) {
  return async (path, body) => {
    const response = await fetch(`${url.replace(/\/$/, '')}${path}`, {
      method: 'POST', headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json', 'User-Agent': 'takomo-agent/0.1' },
      body: JSON.stringify(body), redirect: 'error',
      signal: AbortSignal.any([signal, AbortSignal.timeout(path.endsWith('/claim') ? 35_000 : 10_000)]),
    });
    if (!response.ok) throw new ApiError(response.status);
    return response.status === 204 ? {} : response.json();
  };
}
export async function executeJob(job, { api, serviceId, createCodex, signal, heartbeatMs = 15_000 }) {
  const identity = { service_id: serviceId, attempt_id: job.attempt_id };
  const prefix = `/v1/agent-jobs/${encodeURIComponent(job.id)}`;
  const codex = createCodex();
  let session = {};
  let lost = false;
  let heartbeatChain = Promise.resolve();
  const heartbeat = () => {
    heartbeatChain = heartbeatChain.then(async () => {
      if (lost) throw new Error('Agent job lease was lost.');
      try { await api(`${prefix}/heartbeat`, { ...identity, ...session }); }
      catch (error) { lost = true; codex.close(); throw error; }
    });
    return heartbeatChain;
  };
  const stop = () => { lost = true; codex.close(); };
  signal.addEventListener('abort', stop, { once: true });
  if (signal.aborted) stop();
  const interval = setInterval(() => heartbeat().catch(() => {}), heartbeatMs);
  let result;
  try {
    result = { status: 'completed', ...await codex.run(job, async ids => {
      session = { ...session, ...ids };
      await heartbeat();
    }) };
  } catch (error) {
    result = { status: 'failed', error: error.message.slice(0, 2000), ...session };
  } finally { codex.close(); }
  try {
    if (lost || signal.aborted) return;
    // Only delivery is retried; a Codex turn is never reexecuted automatically.
    for (let attempt = 0; attempt < 5; attempt++) {
      try { await api(`${prefix}/result`, { ...identity, ...result }); return; }
      catch (error) {
        if (lost || error instanceof ApiError && error.status < 500 && error.status !== 429) throw error;
        if (attempt === 4) throw error;
        await sleep(Math.min(500 * 2 ** attempt, 4000), undefined, { signal });
      }
    }
  } finally {
    clearInterval(interval);
    signal.removeEventListener('abort', stop);
    await heartbeatChain.catch(() => {});
  }
}

export async function main() {
  const env = process.env;
  if (!env.TAKOMO_URL || !env.TAKOMO_AGENT_TOKEN) throw new Error('Set TAKOMO_URL and TAKOMO_AGENT_TOKEN (agent:run scope).');
  const url = new URL(env.TAKOMO_URL);
  if (url.protocol !== 'https:' && !(url.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname))) {
    throw new Error('Use HTTPS for remote Takomo connections (HTTP is allowed on loopback).');
  }
  const state = resolve(env.TAKOMO_AGENT_STATE_DIR || join(homedir(), '.takomo-agent'));
  const home = join(state, 'codex');
  const cwd = join(state, 'workspace');
  await mkdir(home, { recursive: true, mode: 0o700 });
  await mkdir(cwd, { recursive: true, mode: 0o700 });
  if ((await readdir(cwd)).length) throw new Error('Agent workspace must be empty. Use a dedicated TAKOMO_AGENT_STATE_DIR.');
  let serviceId = env.TAKOMO_AGENT_SERVICE_ID;
  if (!serviceId) {
    const file = join(state, 'service-id');
    try { serviceId = (await readFile(file, 'utf8')).trim(); }
    catch (error) {
      if (error.code !== 'ENOENT') throw error;
      serviceId = randomUUID();
      await writeFile(file, serviceId, { flag: 'wx', mode: 0o600 });
    }
  }
  const controller = new AbortController();
  const stop = () => controller.abort();
  process.once('SIGINT', stop);
  process.once('SIGTERM', stop);
  const { signal } = controller;
  const api = apiClient(url.href, env.TAKOMO_AGENT_TOKEN, signal);
  let backoff = 500;
  console.log(`Agent service ${serviceId} starting for ${url.origin}.`);
  try {
    while (!signal.aborted) {
      try {
        const { job } = await api('/v1/agent-jobs/claim', { service_id: serviceId, wait_seconds: process.argv.includes('--once') ? 0 : 25 });
        if (job) {
          console.log(`Running job ${job.id}.`);
          await executeJob(job, {
            api, serviceId, signal,
            createCodex: () => new Codex({ executable: env.TAKOMO_CODEX_BIN || 'codex', cwd, home }),
          });
        }
        if (process.argv.includes('--once')) return;
        backoff = 500;
      } catch (error) {
        if (signal.aborted) break;
        if (error instanceof ApiError && [401, 403].includes(error.status)) throw error;
        console.error(error.message);
        if (process.argv.includes('--once')) throw error;
        await sleep(backoff, undefined, { signal });
        backoff = Math.min(backoff * 2, 10_000);
      }
    }
  } finally { process.removeListener('SIGINT', stop); process.removeListener('SIGTERM', stop); }
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch(error => {
    if (error.name !== 'AbortError') { console.error(error.message); process.exitCode = 1; }
  });
}

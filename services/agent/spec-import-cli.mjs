import { mkdir, open, readFile, readdir } from 'node:fs/promises';
import { homedir } from 'node:os';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { randomUUID } from 'node:crypto';
import { parseArgs } from 'node:util';
import { Codex } from './codex.mjs';
import { preflightImport, developmentLimits } from './spec-import-preflight.mjs';
import { IMPORT_KIND, importJob } from './spec-import.mjs';

export async function generateImport({ repository, scope, revision = 'HEAD', limits = developmentLimits, maxSections = 6, output, stateDir, prompt = 'Describe the implemented behavior.' }, createCodex) {
  if (!output) throw new Error('Choose --out for the recoverable draft artifact.');
  if (!Number.isInteger(maxSections) || maxSections < 1 || maxSections > 12) throw new Error('Choose 1–12 sections.');
  if (typeof prompt !== 'string' || prompt.length > 4000) throw new Error('Keep the request under 4000 characters.');
  const preview = await preflightImport({ repository, revision, scope, limits });
  const job = { kind: IMPORT_KIND, repository_ref: { repository: 'import', revision: preview.revision, scope: preview.scope }, repository_limits: preview.limits, max_sections: maxSections, prompt };
  importJob(job);
  const state = resolve(stateDir ?? join(homedir(), '.takomo-agent'));
  const cwd = join(state, 'workspace');
  const home = join(state, 'codex');
  if (!createCodex) {
    await mkdir(cwd, { recursive: true, mode: 0o700 });
    if ((await readdir(cwd)).length) throw new Error('Use the agent service’s empty workspace.');
  }
  // Reserve output before inference. A failed run remains a visible artifact and
  // is never automatically reexecuted by a publish/retry command.
  const file = await open(output, 'wx', 0o600);
  const artifact = { schema_version: 1, status: 'running', request_id: randomUUID(), manifest: preview, max_sections: maxSections };
  const save = async () => { await file.truncate(0); await file.write(`${JSON.stringify(artifact, null, 2)}\n`, 0, 'utf8'); await file.sync(); };
  let codex;
  let interrupted = false;
  const stop = () => { interrupted = true; codex?.close(); };
  process.once('SIGINT', stop); process.once('SIGTERM', stop);
  try {
    await save();
    if (interrupted) throw new Error('Import interrupted before inference.');
    codex = createCodex ? createCodex(job) : new Codex({ executable: process.env.TAKOMO_CODEX_BIN || 'codex', cwd, home, repositories: { import: resolve(repository) }, kind: IMPORT_KIND, timeoutMs: 180_000 });
    const result = await codex.run(job, async session => { artifact.session = { ...artifact.session, ...session }; await save(); });
    artifact.status = 'ready';
    artifact.request = { request_id: artifact.request_id, revision: result.repository_revision, scope: preview.scope, draft: result.draft };
    artifact.evidence = result.evidence;
    artifact.session = { thread_id: result.thread_id, turn_id: result.turn_id };
    await save();
    return artifact;
  } catch (error) {
    artifact.status = 'failed'; artifact.error = error.message;
    await save();
    throw error;
  } finally {
    codex?.close(); await file.close();
    process.removeListener('SIGINT', stop); process.removeListener('SIGTERM', stop);
  }
}

export async function publishImport({ artifact, url, token, mindmap }, fetcher = fetch) {
  if (artifact?.schema_version !== 1 || artifact.status !== 'ready' || !artifact.request) throw new Error('Publish requires a completed, ready draft artifact.');
  if (!token || typeof mindmap !== 'string' || !/^mm-[a-zA-Z0-9]+$/.test(mindmap)) throw new Error('Set TAKOMO_IMPORT_TOKEN and pass a valid --mindmap id.');
  const base = new URL(url);
  if (base.username || base.password || base.search || base.hash || base.pathname !== '/') throw new Error('TAKOMO_URL must be an origin without credentials or a path.');
  if (base.protocol !== 'https:' && !(base.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(base.hostname))) throw new Error('Use HTTPS except on loopback.');
  const response = await fetcher(new URL(`/v1/mindmaps/${mindmap}/codebase-import`, base), {
    method: 'POST', redirect: 'error', headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify(artifact.request), signal: AbortSignal.timeout(30_000),
  });
  const result = await response.json();
  if (!response.ok) throw new Error(`Import was not published (${response.status}): ${result.message ?? result.error?.message ?? 'inspect the target and retry the same artifact'}`);
  return result;
}
const help = `Usage:
  node services/agent/spec-import-cli.mjs generate --repo PATH --include PATH --out draft.json
  node services/agent/spec-import-cli.mjs publish --file draft.json --mindmap mm-ID

Generate uses the existing authenticated agent-service Codex home, one App Server
turn (3 minutes), and scoped repository tools. Defaults: 100 files / 1 MB / 30 tool
calls / 6 sections. Flags: --revision REV, repeatable --include/--exclude,
--max-files N, --max-bytes N, --max-tool-calls N, --max-sections N, --state-dir PATH,
--prompt TEXT. Output is reserved before inference; existing files are never reused.

Publish performs no inference. Set TAKOMO_URL and TAKOMO_IMPORT_TOKEN (read/write/human,
restricted to the target project). The target must be empty. Retry the same artifact
after a transport failure; never generate another draft just to retry publication.
Review generated, unconfirmed sections in the normal document and mindmap views.
`;
export async function main(args = process.argv.slice(2)) {
  const command = args[0];
  if (!command || command === '--help') { process.stdout.write(help); return; }
  const names = command === 'generate' ? ['repo', 'revision', 'out', 'state-dir', 'prompt', 'max-files', 'max-bytes', 'max-tool-calls', 'max-sections'] : ['file', 'mindmap'];
  if (!['generate', 'publish'].includes(command)) throw new Error('Choose generate or publish.');
  const { values } = parseArgs({ args: args.slice(1), options: { ...Object.fromEntries(names.map(key => [key, { type: 'string' }])), ...(command === 'generate' ? { include: { type: 'string', multiple: true }, exclude: { type: 'string', multiple: true } } : {}) } });
  const number = (key, fallback) => {
    if (values[key] === undefined) return fallback;
    if (!/^[1-9]\d*$/.test(values[key])) throw new Error(`--${key} must be a positive integer.`);
    return Number(values[key]);
  };
  if (command === 'generate') {
    const artifact = await generateImport({ repository: values.repo, revision: values.revision,
      scope: { include: values.include, exclude: values.exclude ?? [] }, output: values.out,
      stateDir: values['state-dir'], prompt: values.prompt, maxSections: number('max-sections', 6),
      limits: { max_files: number('max-files',100), max_source_bytes: number('max-bytes',1_000_000), max_tool_calls: number('max-tool-calls',30) },
    });
    process.stdout.write(`Draft ready: ${values.out} (${artifact.request.draft.sections.length} sections). Publish it into an empty specification for review.\n`);
  } else {
    if (!values.file) throw new Error('Pass --file with a ready draft artifact.');
    const artifact = JSON.parse(await readFile(values.file, 'utf8'));
    const result = await publishImport({ artifact, url: process.env.TAKOMO_URL, token: process.env.TAKOMO_IMPORT_TOKEN, mindmap: values.mindmap });
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  }
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) main().catch(error => { process.stderr.write(`${error.message}\n`); process.exitCode = 1; });

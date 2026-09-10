import { writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { parseArgs } from 'node:util';
import { openRepository } from './repository.mjs';
import { repositoryLimits, repositoryScope } from './repository-scope.mjs';

export const developmentLimits = Object.freeze({ max_files: 100, max_source_bytes: 1_000_000, max_tool_calls: 30 });

// A local, no-inference inventory. The future queued import consumes the same scope contract.
export async function preflightImport({ repository, revision = 'HEAD', scope, limits = developmentLimits }) {
  if (typeof repository !== 'string' || !repository) throw new Error('Choose a local Git repository path.');
  if (scope === undefined) throw new Error('Choose an explicit include scope; use ["."] deliberately for a whole-repository preflight.');
  const checkedScope = repositoryScope(scope);
  repositoryLimits(limits);
  const checkedLimits = repositoryLimits({ ...developmentLimits, ...limits });
  const repo = await openRepository({ repository_ref: { repository: 'preflight', revision, scope: checkedScope } },
    { preflight: resolve(repository) }, checkedLimits);
  const page = JSON.parse(await repo.call('repository_files', {}));
  return {
    schema_version: 1, kind: 'spec_import_preflight',
    ...repo.manifest(), files: page.files, files_truncated: page.truncated,
    inference: { model_calls: 0, estimated_tokens: null, estimated_cost: null },
    next_step: 'Use this pinned revision and scope for the import. This inventory neither analyzes behavior nor creates document sections.',
  };
}

const help = `Usage: node services/agent/spec-import-preflight.mjs --repo PATH --include PATH [options]

Inventory only; no Codex process, model calls, network, or document writes.

  --repo PATH          Local Git repository (required)
  --revision REV       Commit/ref to pin; defaults to HEAD
  --include PATH       Literal repository-relative file/directory; repeatable
  --exclude PATH       Excluded file/directory; repeatable; exclusion wins
  --max-files N        Eligible file ceiling; default 100 (maximum 100000)
  --max-bytes N        Eligible source-byte ceiling; default 1000000
  --max-tool-calls N   Recorded later-run tool-call ceiling; default 30 (maximum 100)
  --out PATH           Also save JSON, refusing to overwrite an existing file
  --help               Print this help

Paths are exact files or directory prefixes, not globs. Use --include . explicitly
for the whole repository. Budgets fail before inference rather than sampling files.
JSON includes complete counts and at most 200 selected paths. Byte counts are not
token/cost estimates; binary content is only identified when read or searched.
`;

export async function main(args = process.argv.slice(2)) {
  const { values } = parseArgs({ args, options: {
    repo: { type: 'string' }, revision: { type: 'string' },
    include: { type: 'string', multiple: true }, exclude: { type: 'string', multiple: true },
    'max-files': { type: 'string' }, 'max-bytes': { type: 'string' }, 'max-tool-calls': { type: 'string' },
    out: { type: 'string' }, help: { type: 'boolean' },
  } });
  if (values.help) { process.stdout.write(help); return; }
  if (!values.repo || !values.include?.length) throw new Error('Pass --repo and at least one --include path. Use --help for examples and limits.');
  const integer = (key, fallback) => {
    if (values[key] === undefined) return fallback;
    if (!/^[1-9]\d*$/.test(values[key])) throw new Error(`--${key} must be a positive integer.`);
    return Number(values[key]);
  };
  const result = await preflightImport({ repository: values.repo, revision: values.revision,
    scope: { include: values.include, exclude: values.exclude ?? [] },
    limits: { max_files: integer('max-files', developmentLimits.max_files),
      max_source_bytes: integer('max-bytes', developmentLimits.max_source_bytes),
      max_tool_calls: integer('max-tool-calls', developmentLimits.max_tool_calls) },
  });
  const json = `${JSON.stringify(result, null, 2)}\n`;
  if (values.out) await writeFile(values.out, json, { flag: 'wx', mode: 0o600 });
  process.stdout.write(json);
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch(error => { process.stderr.write(`${error.message}\n`); process.exitCode = 1; });
}

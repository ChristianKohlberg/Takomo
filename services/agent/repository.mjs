import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { isAbsolute } from 'node:path';
const exec = promisify(execFile);

// Only Git object reads: no checkout, hooks, repository scripts, symlink following, or network.
export async function openRepository(job, repositories) {
  const key = job.repository_ref?.repository;
  const cwd = Object.hasOwn(repositories, key) && repositories[key];
  if (typeof cwd !== 'string' || !isAbsolute(cwd)) throw new Error('Research repository is not configured on this worker.');
  const git = async args => (await exec('git', ['--no-replace-objects', '-c', 'core.fsmonitor=false', ...args], {
    cwd, timeout: 10_000, maxBuffer: 2_000_000,
    env: { PATH: process.env.PATH, GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null', GIT_TERMINAL_PROMPT: '0' },
  })).stdout;
  const requested = job.repository_ref?.revision || 'HEAD';
  if (typeof requested !== 'string' || !/^[a-zA-Z0-9_./-]{1,200}$/.test(requested) || requested.startsWith('-')) throw new Error('Invalid repository revision.');
  const revision = (await git(['rev-parse', '--verify', '--end-of-options', `${requested}^{commit}`])).trim();
  if (!/^[a-f0-9]{40,64}$/.test(revision)) throw new Error('Repository revision did not resolve to a commit.');
  const entries = (await git(['ls-tree', '-rz', '--full-tree', revision])).split('\0').filter(Boolean);
  const files = new Map(entries.flatMap(entry => {
    const match = /^(100644|100755) blob ([a-f0-9]+)\t([\s\S]+)$/.exec(entry);
    return match ? [[match[3], match[2]]] : [];
  }));
  let calls = 0;
  const evidence = [];
  let evidenceTruncated = false;
  function record(item) {
    if (Buffer.byteLength(JSON.stringify([...evidence, item]), 'utf8') <= 48_000) evidence.push(item);
    else evidenceTruncated = true;
  }
  return {
    revision, evidence,
    progress: () => ({ inspected: [...evidence], truncated: evidenceTruncated, runtime_reproduced: false }),
    async call(name, args) {
      if (++calls > 100) throw new Error('Research tool budget exhausted (100 calls).');
      if (!args || typeof args !== 'object') throw new Error('Expected tool arguments.');
      if (name === 'repository_files') {
        const query = typeof args.query === 'string' ? args.query : '';
        const matches = [...files.keys()].filter(path => path.includes(query));
        return JSON.stringify({ revision, files: matches.slice(0, 200), total: matches.length, truncated: matches.length > 200 });
      }
      if (name === 'repository_search') {
        if (typeof args.query !== 'string' || !args.query || args.query.length > 200) throw new Error('Use a literal query of 1–200 characters.');
        let matches;
        try { matches = await git(['grep', '-I', '-n', '-F', '-e', args.query, revision, '--']); }
        catch (error) { if (error.code === 1) matches = ''; else throw new Error('Search exceeded its resource limit; narrow the query.'); }
        const lines = matches.split('\n').filter(Boolean);
        return JSON.stringify({ revision, matches: lines.slice(0, 100), total: lines.length, truncated: lines.length > 100 });
      }
      if (name !== 'repository_read') throw new Error('Unsupported research tool.');
      if (!files.has(args.path)) throw new Error('Choose a regular tracked file from repository_files.');
      const start = args.start_line ?? 1;
      if (!Number.isSafeInteger(start) || start < 1) throw new Error('start_line must be a positive integer.');
      const object = files.get(args.path);
      if (Number(await git(['cat-file', '-s', object])) > 1_000_000) throw new Error('File exceeds the 1 MB research limit.');
      const content = await git(['cat-file', 'blob', object]);
      if (content.includes('\0')) throw new Error('Binary files cannot be researched.');
      const lines = content.split('\n');
      const selected = lines.slice(start - 1, start + 199).map((line, index) => `${start + index}: ${line}`).join('\n');
      const text = selected.slice(0, 24_000);
      record({ path: args.path, start_line: start, end_line: start + text.split('\n').length - 1, revision });
      return JSON.stringify({ revision, path: args.path, total_lines: lines.length, content: text, truncated: selected.length > text.length || start + 199 < lines.length });
    },
  };
}
export const repositoryTools = [
  { type: 'function', name: 'repository_search', description: 'Search tracked text content at the pinned commit for a literal substring, with file and line references. Up to 100 matches; narrow queries when truncated.', inputSchema: { type: 'object', properties: { query: { type: 'string', minLength: 1, maxLength: 200 } }, required: ['query'], additionalProperties: false } },
  { type: 'function', name: 'repository_files', description: 'List up to 200 tracked regular files at the pinned commit. Filter paths by a literal substring. Reports total matches.', inputSchema: { type: 'object', properties: { query: { type: 'string' } }, additionalProperties: false } },
  { type: 'function', name: 'repository_read', description: 'Read up to 200 numbered lines of a tracked text file at the pinned commit. No execution. Return later lines by setting start_line.', inputSchema: { type: 'object', properties: { path: { type: 'string' }, start_line: { type: 'integer', minimum: 1 } }, required: ['path'], additionalProperties: false } },
];

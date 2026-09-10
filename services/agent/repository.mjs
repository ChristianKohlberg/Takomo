import { createHash } from 'node:crypto';
import { isAbsolute } from 'node:path';
import { StringDecoder } from 'node:string_decoder';
import { repositoryGit } from './repository-git.mjs';
import { repositoryLimits, repositoryPath, repositoryScope, underPath } from './repository-scope.mjs';

const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const FILE_BYTES = 1_000_000;
const READ_BYTES = 24_000;
function byteSlice(text, limit) {
  const buf = Buffer.from(text);
  if (buf.length <= limit) return text;
  let end = limit;
  while (end && (buf[end] & 0xc0) === 0x80) end--;
  return buf.subarray(0, end).toString('utf8');
}
function checkArgs(args, fields) {
  if (!args || typeof args !== 'object' || Array.isArray(args) || Object.keys(args).some(key => !fields.includes(key))) throw new Error('Unsupported repository tool arguments.');
}
function offsetFor(cursor, fingerprint) {
  if (cursor === undefined) return 0;
  try {
    if (typeof cursor !== 'string' || cursor.length > 300) throw new Error();
    const data = JSON.parse(Buffer.from(cursor, 'base64url').toString('utf8'));
    if (data.key !== fingerprint || !Number.isSafeInteger(data.offset) || data.offset < 0) throw new Error();
    return data.offset;
  } catch { throw new Error('Invalid cursor: use the returned next_cursor with the same tool, scope, revision and query.'); }
}
const cursorFor = (offset, key) => Buffer.from(JSON.stringify({ offset, key })).toString('base64url');

// Only Git object reads: no checkout, hooks, repository scripts, symlinks, or network.
export async function openRepository(job, repositories, options = {}) {
  const key = job.repository_ref?.repository;
  const cwd = Object.hasOwn(repositories, key) && repositories[key];
  if (typeof cwd !== 'string' || !isAbsolute(cwd)) throw new Error('Research repository is not configured on this worker.');
  const scope = repositoryScope(job.repository_ref?.scope);
  const limits = repositoryLimits(options);
  const git = repositoryGit(cwd);
  const requested = job.repository_ref?.revision ?? 'HEAD';
  if (typeof requested !== 'string' || !/^[a-zA-Z0-9_./-]{1,200}$/.test(requested) || requested.startsWith('-')) throw new Error('Invalid repository revision.');
  const revision = (await git(['rev-parse', '--verify', '--end-of-options', `${requested}^{commit}`])).trim();
  if (!/^[a-f0-9]{40,64}$/.test(revision)) throw new Error('Repository revision did not resolve to a commit.');
  const files = new Map();
  const counts = { tracked_in_included_paths: 0, excluded_by_path: 0, non_regular: 0, unsupported_path: 0, oversized: 0, selected_files: 0, selected_bytes: 0 };
  const matched = new Set();
  let pending = Buffer.alloc(0);
  await git(['ls-tree', '-rlz', '--full-tree', revision, '--', ...scope.include.filter(path => path !== '.')], {
    maxBytes: 32_000_000,
    onChunk(chunk) {
      pending = Buffer.concat([pending, chunk]);
      let end;
      while ((end = pending.indexOf(0)) !== -1) {
        const raw = pending.subarray(0, end);
        pending = pending.subarray(end + 1);
        const entry = raw.toString('utf8');
        const match = /^(\d+) (\w+) ([a-f0-9]+)\s+(\d+|-|BAD)\t([\s\S]+)$/.exec(entry);
        if (!match) throw new Error('Malformed repository inventory.');
        const [, mode, type, blob, sizeText, path] = match;
        if (!scope.include.some(prefix => underPath(path, prefix))) continue;
        counts.tracked_in_included_paths++;
        for (const prefix of scope.include) if (underPath(path, prefix)) matched.add(prefix);
        if (scope.exclude.some(prefix => underPath(path, prefix))) { counts.excluded_by_path++; continue; }
        if (!['100644', '100755'].includes(mode) || type !== 'blob') { counts.non_regular++; continue; }
        try { repositoryPath(path); if (!raw.equals(Buffer.from(entry))) throw new Error(); }
        catch { counts.unsupported_path++; continue; }
        if (sizeText === '-' || sizeText === 'BAD') throw new Error('Selected source object is unavailable locally. Prepare the pinned source before analysis.');
        const bytes = Number(sizeText);
        if (bytes > FILE_BYTES) { counts.oversized++; continue; }
        counts.selected_files++;
        counts.selected_bytes += bytes;
        if (counts.selected_files > limits.max_files || counts.selected_bytes > limits.max_source_bytes) {
          throw new Error(`Repository scope exceeds its budget (${counts.selected_files} eligible files / ${counts.selected_bytes} bytes seen; limits ${limits.max_files} files / ${limits.max_source_bytes} bytes). Narrow include paths or explicitly raise the preflight limits; no files were sampled.`);
        }
        files.set(path, { blob, bytes });
      }
    },
  });
  if (pending.length) throw new Error('Incomplete repository inventory.');
  const unmatched = scope.include.filter(prefix => !matched.has(prefix));
  if (job.repository_ref?.scope && unmatched.length) throw new Error(`Scope includes no tracked entries for: ${unmatched.join(', ')}. Check paths at the pinned revision.`);
  if (job.repository_ref?.scope && !files.size) throw new Error('Repository scope contains no eligible regular files after exclusions.');
  const paths = [...files.keys()].sort();
  const inventoryId = hash({ revision, scope, paths });
  let calls = 0;
  const evidence = [];
  let evidenceTruncated = false;
  const manifest = () => ({ revision, scope: structuredClone(scope), inventory_id: inventoryId, limits: { ...limits }, counts: { ...counts },
    coverage: { whole_repository: scope.include.includes('.') && !scope.exclude.length, content_inspected: false,
      skipped_files: counts.excluded_by_path + counts.non_regular + counts.unsupported_path + counts.oversized,
      note: 'Inventory counts describe included paths only. Binary content is detected on read/search. Metadata enumeration is not behavioral coverage.' } });
  function record(item) {
    if (Buffer.byteLength(JSON.stringify([...evidence, item]), 'utf8') <= 48_000) evidence.push(item);
    else evidenceTruncated = true;
  }
  return {
    revision, evidence, manifest,
    progress: () => ({ inspected: [...evidence], truncated: evidenceTruncated, runtime_reproduced: false }),
    async call(name, args) {
      if (++calls > limits.max_tool_calls) throw new Error(`Research tool budget exhausted (${limits.max_tool_calls} calls).`);
      if (name === 'repository_files' || name === 'repository_search') {
        checkArgs(args, ['query', 'path_prefix', 'cursor']);
        const query = args.query ?? '';
        if (typeof query !== 'string' || query.length > 200 || query.includes('\0') || (name === 'repository_search' && (!query || /[\r\n]/.test(query)))) throw new Error('Use a literal query of 1–200 characters for search, without line breaks or NUL.');
        const prefix = args.path_prefix === undefined ? '.' : repositoryPath(args.path_prefix, { root: true });
        const selected = paths.filter(path => underPath(path, prefix) && (name !== 'repository_files' || path.includes(query)));
        const fingerprint = hash({ inventoryId, name, query, prefix });
        const offset = offsetFor(args.cursor, fingerprint);
        if (name === 'repository_files') {
          const page = selected.slice(offset, offset + 200);
          const next = offset + page.length;
          return JSON.stringify({ revision, files: page, total: selected.length, truncated: next < selected.length,
            next_cursor: next < selected.length ? cursorFor(next, fingerprint) : null });
        }
        let total = 0;
        const matches = [];
        let textTruncated = false;
        let scannedBytes = 0;
        const started = performance.now();
        // Explicit regular-file path batches prevent search following excluded files/symlinks.
        for (let i = 0; i < selected.length; i += 64) {
          if (performance.now() - started > 10_000) throw new Error('Search exceeded its time budget; narrow path_prefix or the run scope.');
          let buffer = '';
          const decoder = new StringDecoder('utf8');
          await git(['grep', '--no-textconv', '-I', '-n', '-z', '-F', '-e', query, revision, '--', ...selected.slice(i, i + 64)], {
            okCodes: [0, 1], maxBytes: 16_000_000,
            onChunk(chunk) {
              scannedBytes += chunk.length;
              if (scannedBytes > 16_000_000) throw new Error('Search exceeded its output budget; narrow path_prefix or the query.');
              buffer += decoder.write(chunk);
              while (true) {
                const pathEnd = buffer.indexOf('\0');
                const numberEnd = pathEnd < 0 ? -1 : buffer.indexOf('\0', pathEnd + 1);
                const lineEnd = numberEnd < 0 ? -1 : buffer.indexOf('\n', numberEnd + 1);
                if (lineEnd < 0) break;
                if (total >= offset && matches.length < 100) {
                  const reference = `${buffer.slice(0, pathEnd)}:${buffer.slice(pathEnd + 1, numberEnd)}:`;
                  const source = buffer.slice(numberEnd + 1, lineEnd);
                  const snippet = byteSlice(source, 1000);
                  textTruncated ||= snippet !== source;
                  matches.push(reference + snippet);
                }
                total++;
                buffer = buffer.slice(lineEnd + 1);
              }
            },
          });
          buffer += decoder.end();
          if (buffer.length) throw new Error('Incomplete search result; narrow the query.');
        }
        const next = offset + matches.length;
        return JSON.stringify({ revision, matches, total, truncated: next < total,
          next_cursor: next < total ? cursorFor(next, fingerprint) : null, text_truncated: textTruncated });
      }
      if (name !== 'repository_read') throw new Error('Unsupported research tool.');
      checkArgs(args, ['path', 'start_line']);
      if (!files.has(args.path)) throw new Error('Choose a regular tracked file from repository_files within the run scope (at most 1 MB).');
      const start = args.start_line ?? 1;
      if (!Number.isSafeInteger(start) || start < 1) throw new Error('start_line must be a positive integer.');
      const { blob } = files.get(args.path);
      const content = await git(['cat-file', 'blob', blob]);
      if (content.includes('\0')) throw new Error('Binary files cannot be researched.');
      const lines = content.split('\n');
      if (lines.at(-1) === '') lines.pop();
      if (start > Math.max(1, lines.length)) throw new Error(`start_line exceeds this file's ${lines.length} lines.`);
      const page = [];
      let usedBytes = 0;
      let clipped = false;
      for (let i = start - 1; i < Math.min(start + 199, lines.length); i++) {
        const numbered = `${i + 1}: ${lines[i]}`;
        const available = READ_BYTES - usedBytes - (page.length ? 1 : 0);
        if (Buffer.byteLength(numbered) > available) {
          if (!page.length) { page.push(byteSlice(numbered, available)); clipped = true; }
          break;
        }
        page.push(numbered);
        usedBytes += Buffer.byteLength(numbered) + (page.length > 1 ? 1 : 0);
      }
      const end = page.length ? start + page.length - 1 : null;
      if (end !== null) record({ path: args.path, start_line: start, end_line: end, revision, ...(clipped ? { line_truncated: true } : {}) });
      return JSON.stringify({ revision, path: args.path, total_lines: lines.length, content: page.join('\n'),
        end_line: end, next_start_line: end !== null && end < lines.length ? end + 1 : null,
        line_truncated: clipped, truncated: clipped || (end !== null && end < lines.length) });
    },
  };
}
const paged = { query: { type: 'string', maxLength: 200 }, path_prefix: { type: 'string', description: 'Literal file/directory path within the run scope.' }, cursor: { type: 'string', description: 'next_cursor from the same tool/query; omit for first page.' } };
export const repositoryTools = [
  { type: 'function', name: 'repository_search', description: 'Search eligible scoped files at the pinned commit for a literal substring. Up to 100 matches per page. Continue with next_cursor, narrow with path_prefix. Long snippets report text_truncated; read the source for full evidence.', inputSchema: { type: 'object', properties: { ...paged, query: { type: 'string', minLength: 1, maxLength: 200 } }, required: ['query'], additionalProperties: false } },
  { type: 'function', name: 'repository_files', description: 'List eligible regular files within the run scope at the pinned commit, up to 200 per page. Continue with next_cursor. query is a literal path substring; path_prefix narrows to a file or directory.', inputSchema: { type: 'object', properties: paged, additionalProperties: false } },
  { type: 'function', name: 'repository_read', description: 'Read a scoped tracked text file, at most 200 numbered lines / 24 KB. Continue with next_start_line. line_truncated means an oversized source line was clipped. No execution.', inputSchema: { type: 'object', properties: { path: { type: 'string' }, start_line: { type: 'integer', minimum: 1 } }, required: ['path'], additionalProperties: false } },
];

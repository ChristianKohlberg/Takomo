import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, mkdir, rm, symlink, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { openRepository } from '../repository.mjs';
import { repositoryScope, repositoryLimits } from '../repository-scope.mjs';
import { preflightImport } from '../spec-import-preflight.mjs';

async function fixture(t, entries) {
  const cwd = await mkdtemp(join(tmpdir(), 'takomo-import-scope-'));
  t.after(() => rm(cwd, { recursive: true, force: true }));
  const git = args => execFileSync('git', args, { cwd, stdio: 'pipe' }).toString().trim();
  git(['init']);
  for (const [path, content] of Object.entries(entries)) {
    await mkdir(dirname(join(cwd, path)), { recursive: true });
    await writeFile(join(cwd, path), content);
  }
  const commit = () => {
    git(['add', '.']);
    git(['-c', 'user.name=Test', '-c', 'user.email=test@example.com', '-c', 'core.hooksPath=/dev/null', 'commit', '-m', 'fixture']);
    return git(['rev-parse', 'HEAD']);
  };
  const revision = commit();
  const open = (scope, limits) => openRepository({ repository_ref: { repository: 'fixture', revision, ...(scope ? { scope } : {}) } }, { fixture: cwd }, limits);
  return { cwd, git, commit, revision, open };
}
const call = async (repo, name, args = {}) => JSON.parse(await repo.call(`repository_${name}`, args));

test('scope is explicit, literal, normalized and bounded', () => {
  for (const scope of [null, [], {}, { include: [] }, { include: ['src'], typo: true }, { include: ['src'], exclude: null },
    ...['/etc', '../src', 'src/../other', 'src//other', './src', 'src/', 'C:/src', 'src\\other', 'src\nother'].map(path => ({ include: [path] }))]) {
    assert.throws(() => repositoryScope(scope));
  }
  assert.deepEqual(repositoryScope({ include: ['src/z', 'src', 'src'], exclude: ['src/test/a', 'src/test'] }), { include: ['src'], exclude: ['src/test'] });
  assert.deepEqual(repositoryScope({ include: ['.', 'src'] }), { include: ['.'], exclude: [] });
  assert.throws(() => repositoryLimits({ max_files: 0 }));
  assert.throws(() => repositoryLimits({ max_tool_calls: 101 }));
  assert.throws(() => repositoryLimits({ max_bytes: 100 }));
});

test('scope applies identically to listing, search and direct reads; prefixes do not leak siblings', async t => {
  const f = await fixture(t, { 'src/a.js': 'needle visible\n', 'src/private/key': 'needle private\n', 'src2/a.js': 'needle sibling\n', 'outside': 'needle outside\n' });
  const repo = await f.open({ include: ['src'], exclude: ['src/private'] });
  assert.deepEqual((await call(repo, 'files')).files, ['src/a.js']);
  const search = await call(repo, 'search', { query: 'needle' });
  assert.equal(search.total, 1);
  assert.match(search.matches[0], /src\/a.js:1:needle visible/);
  assert.equal((await call(repo, 'search', { query: 'needle', path_prefix: '.' })).total, 1);
  assert.equal((await call(repo, 'search', { query: 'needle', path_prefix: 'src2' })).total, 0);
  for (const path of ['src/private/key', 'src2/a.js', 'outside', '../outside']) await assert.rejects(call(repo, 'read', { path }), /regular tracked/);
  assert.equal(repo.manifest().counts.excluded_by_path, 1);
  assert.equal(repo.manifest().coverage.whole_repository, false);
  assert.equal(repo.manifest().counts.tracked_in_included_paths, 2);
});

test('Git metacharacters in scope and filenames are literal, not pathspec patterns', async t => {
  const f = await fixture(t, { '[module]/a': 'needle\n', 'm/a': 'needle forbidden\n', ':magic': 'needle literal\n', '-flag': 'needle flag\n' });
  for (const path of ['[module]', ':magic', '-flag']) {
    const repo = await f.open({ include: [path] });
    assert.equal((await call(repo, 'files')).total, 1);
    assert.equal((await call(repo, 'search', { query: 'needle' })).total, 1);
  }
});

test('explicit empty or misspelled scope fails instead of falling back to the entire repository', async t => {
  const f = await fixture(t, { 'src/a': 'hello' });
  await assert.rejects(f.open({ include: ['missing'] }), /no tracked entries/);
  await assert.rejects(f.open({ include: ['src'], exclude: ['src'] }), /no eligible/);
  await assert.rejects(f.open({ include: ['src', 'missing'] }), /missing/);
  const all = await f.open({ include: ['src', '.'] });
  assert.equal((await call(all, 'files')).total, 1);
});

test('scope is pinned to committed source and reports skipped nonregular/oversized paths', async t => {
  const f = await fixture(t, { 'src/a': 'committed needle\n', 'src/big': 'x'.repeat(1_000_001), 'src/binary': Buffer.from([0, 1, 2]), 'src/line\nbreak': 'unaddressable' });
  await symlink('/etc/passwd', join(f.cwd, 'src/link'));
  const revision = f.commit();
  await writeFile(join(f.cwd, 'src/a'), 'dirty secret');
  await writeFile(join(f.cwd, 'src/untracked'), 'untracked secret');
  const repo = await openRepository({ repository_ref: { repository: 'f', revision, scope: { include: ['src'] } } }, { f: f.cwd });
  assert.deepEqual((await call(repo, 'files')).files, ['src/a', 'src/binary']);
  assert.equal((await call(repo, 'search', { query: 'secret' })).total, 0);
  assert.match((await call(repo, 'read', { path: 'src/a' })).content, /committed/);
  await assert.rejects(call(repo, 'read', { path: 'src/binary' }), /Binary/);
  await assert.rejects(call(repo, 'read', { path: 'src/link' }), /regular tracked/);
  assert.deepEqual(repo.manifest().counts, { tracked_in_included_paths: 5, excluded_by_path: 0, non_regular: 1, unsupported_path: 1, oversized: 1, selected_files: 2, selected_bytes: 20 });
});

test('pagination enumerates every scoped file and match without duplication', async t => {
  const entries = Object.fromEntries(Array.from({ length: 231 }, (_, i) => [`src/f${String(i).padStart(3, '0')}.js`, `needle ${i}\n`]));
  const f = await fixture(t, entries);
  const repo = await f.open({ include: ['src'] });
  for (const tool of ['files', 'search']) {
    const collected = [];
    let cursor;
    do {
      const page = await call(repo, tool, { ...(tool === 'search' ? { query: 'needle' } : {}), ...(cursor ? { cursor } : {}) });
      assert.equal(page.total, 231);
      collected.push(...page[tool === 'files' ? 'files' : 'matches']);
      cursor = page.next_cursor;
      assert.equal(page.truncated, cursor !== null);
    } while (cursor);
    assert.equal(collected.length, 231);
    assert.equal(new Set(collected).size, 231);
  }
  const page = await call(repo, 'files');
  await assert.rejects(call(repo, 'files', { query: 'f0', cursor: page.next_cursor }), /Invalid cursor/);
  const other = await f.open({ include: ['src'], exclude: ['src/f230.js'] });
  await assert.rejects(call(other, 'files', { cursor: page.next_cursor }), /Invalid cursor/);
  await assert.rejects(call(repo, 'search', { query: 'needle', cursor: page.next_cursor }), /Invalid cursor/);
  await assert.rejects(call(repo, 'files', { cursor: 'bad' }), /Invalid cursor/);
});

test('search streams more than the old 2 MB limit while returning bounded excerpts', async t => {
  const source = `${'needle '.repeat(140)}\n`.repeat(900);
  const f = await fixture(t, { 'src/a': source, 'src/b': source, 'src/c': source });
  const repo = await f.open({ include: ['src'] });
  const result = await call(repo, 'search', { query: 'needle' });
  assert.equal(result.total, 2700);
  assert.equal(result.matches.length, 100);
  assert.ok(Buffer.byteLength(JSON.stringify(result)) < 120_000);
  assert.ok(result.next_cursor);
});

test('UTF-8 reads resume at exact line boundaries with no lost lines; oversized lines are explicit', async t => {
  const lines = Array.from({ length: 230 }, (_, i) => `${i} ${'😀'.repeat(60)}`);
  const f = await fixture(t, { 'src/unicode': `${lines.join('\n')}\n`, 'src/huge-line': `${'😀'.repeat(7000)}\nlast\n`, 'src/empty': '' });
  const repo = await f.open({ include: ['src'] });
  const seen = [];
  let start = 1;
  do {
    const page = await call(repo, 'read', { path: 'src/unicode', start_line: start });
    assert.ok(Buffer.byteLength(page.content) <= 24_000);
    assert.equal(page.line_truncated, false);
    assert.doesNotMatch(page.content, /\uFFFD/);
    seen.push(...page.content.split('\n').map(line => line.replace(/^\d+: /, '')));
    start = page.next_start_line;
  } while (start);
  assert.deepEqual(seen, lines);
  const clipped = await call(repo, 'read', { path: 'src/huge-line' });
  assert.equal(clipped.line_truncated, true);
  assert.equal(clipped.end_line, 1);
  assert.equal(clipped.next_start_line, 2);
  assert.doesNotMatch(clipped.content, /\uFFFD/);
  assert.equal((await call(repo, 'read', { path: 'src/empty' })).end_line, null);
  await assert.rejects(call(repo, 'read', { path: 'src/unicode', start_line: 231 }), /exceeds/);
});

test('file/byte budgets fail before analysis; host tool budget cannot be raised by a call', async t => {
  const f = await fixture(t, { 'a': '1234', 'b': '5678' });
  await assert.rejects(f.open({ include: ['.'] }, { max_files: 1 }), /exceeds its budget/);
  await assert.rejects(f.open({ include: ['.'] }, { max_source_bytes: 7 }), /exceeds its budget/);
  const repo = await f.open({ include: ['.'] }, { max_tool_calls: 2 });
  await assert.rejects(call(repo, 'files', { max_tool_calls: 100 }), /Unsupported/);
  assert.equal((await call(repo, 'files')).total, 2);
  await assert.rejects(call(repo, 'files'), /budget exhausted \(2 calls\)/);
});

test('preflight requires scope, pins revision and reports no inference; CLI refuses overwriting output', async t => {
  const f = await fixture(t, { 'src/a': 'needle', 'outside': 'secret' });
  await assert.rejects(preflightImport({ repository: f.cwd }), /explicit include/);
  const report = await preflightImport({ repository: f.cwd, scope: { include: ['src'] } });
  assert.equal(report.revision, f.revision);
  assert.deepEqual(report.files, ['src/a']);
  assert.equal(report.inference.model_calls, 0);
  assert.equal(report.inference.estimated_cost, null);
  assert.equal(report.counts.selected_bytes, 6);
  assert.equal(report.coverage.content_inspected, false);
  const script = fileURLToPath(new URL('../spec-import-preflight.mjs', import.meta.url));
  const out = join(f.cwd, 'report.json');
  const args = [script, '--repo', f.cwd, '--include', 'src', '--out', out];
  const result = JSON.parse(execFileSync(process.execPath, args, { encoding: 'utf8' }));
  assert.deepEqual(JSON.parse(await readFile(out, 'utf8')), result);
  assert.throws(() => execFileSync(process.execPath, args, { stdio: 'pipe' }), /EEXIST/);
  assert.deepEqual(JSON.parse(await readFile(out, 'utf8')), result);
  assert.throws(() => execFileSync(process.execPath, [script, '--repo', f.cwd], { stdio: 'pipe' }), /--include/);
});

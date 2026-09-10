// Deterministic App Server protocol fixture; never starts a live provider.
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, copyFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { parseArgs } from 'node:util';
import { Codex, configArgs, profileFor } from '../codex.mjs';
import { IMPORT_KIND } from '../spec-import.mjs';
import { generateImport, publishImport } from '../spec-import-cli.mjs';

const { values } = parseArgs({ options: { out: { type: 'string' }, mindmap: { type: 'string' } } });
if (!values.out) throw new Error('Pass --out for the simulated draft artifact.');
// This test helper can publish only to an explicitly supplied loopback test server.
if (values.mindmap) {
  const url = new URL(process.env.TAKOMO_URL);
  assert.equal(url.protocol, 'http:');
  assert.ok(['127.0.0.1', '[::1]', 'localhost'].includes(url.hostname));
}
const repo = await mkdtemp(join(tmpdir(), 'takomo-checkout-smoke-'));
try {
  const scope = 'examples/extraction-fixture/checkout.mjs';
  await mkdir(join(repo, 'examples/extraction-fixture'), { recursive: true });
  await copyFile(fileURLToPath(new URL('../../../examples/extraction-fixture/checkout.mjs', import.meta.url)), join(repo, scope));
  await writeFile(join(repo, 'outside.txt'), 'This file must not be read.\n');
  const git = args => execFileSync('git', args, { cwd: repo, stdio: 'pipe' });
  git(['init']); git(['add', '.']);
  git(['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', '-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgsign=false', 'commit', '-m', 'Disposable checkout fixture']);
  const artifact = await generateImport({ repository: repo, scope: { include: [scope], exclude: [] },
    limits: { max_files: 1, max_source_bytes: 5000, max_tool_calls: 2 }, maxSections: 3, output: values.out,
  }, () => new Codex({ executable: process.execPath,
    args: [fileURLToPath(new URL('./fake-import.mjs', import.meta.url)), '--checkout-fixture', ...configArgs(profileFor(IMPORT_KIND))],
    cwd: repo, home: repo, repositories: { import: repo }, kind: IMPORT_KIND, timeoutMs: 10_000,
  }));
  assert.equal(artifact.manifest.counts.selected_files, 1);
  assert.deepEqual(artifact.evidence.inspected.map(item => item.path), [scope]);
  assert.equal(artifact.request.draft.sections.length, 3);
  if (values.mindmap) {
    const options = { artifact, url: process.env.TAKOMO_URL, token: process.env.TAKOMO_IMPORT_TOKEN, mindmap: values.mindmap };
    const result = await publishImport(options);
    assert.deepEqual(await publishImport(options), result);
    assert.equal(result.reviewed, false);
  }
  process.stdout.write(`PASS: one scoped file, three simulated sections. ${values.mindmap ? 'HTTP publication and retry verified.' : 'Generation only; nothing published.'} No paid inference.\n`);
} finally {
  await rm(repo, { recursive: true, force: true });
}

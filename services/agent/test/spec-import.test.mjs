import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { Codex, configArgs, profileFor } from '../codex.mjs';
import { IMPORT_KIND } from '../spec-import.mjs';
import { generateImport, publishImport } from '../spec-import-cli.mjs';

async function fixture(t) {
  const repo = await mkdtemp(join(tmpdir(),'takomo-import-mvp-'));
  t.after(()=>rm(repo,{recursive:true,force:true}));
  const git=args=>execFileSync('git',args,{cwd:repo,stdio:'pipe'}).toString().trim();
  git(['init']);await mkdir(join(repo,'src')); await writeFile(join(repo,'src/sample.js'),'export const enabled = true;\n');
  await writeFile(join(repo,'outside.js'),'private outside scope\n');git(['add','.']);
  git(['-c','user.name=Test','-c','user.email=test@example.com','-c','core.hooksPath=/dev/null','commit','-m','fixture']);
  const revision=git(['rev-parse','HEAD']);
  const job={kind:IMPORT_KIND,repository_ref:{repository:'import',revision,scope:{include:['src'],exclude:[]}},repository_limits:{max_files:5,max_source_bytes:10000,max_tool_calls:5},max_sections:4,prompt:'Draft'};
  const createCodex=()=>new Codex({executable:process.execPath,args:[fileURLToPath(new URL('./fake-import.mjs',import.meta.url)),...configArgs(profileFor(IMPORT_KIND))],cwd:repo,home:repo,repositories:{import:repo},kind:IMPORT_KIND,timeoutMs:500});
  return {repo,job,revision,createCodex};
}
test('App Server import reads scoped evidence and returns a structured hierarchy',async t=>{
  const f=await fixture(t);const codex=f.createCodex();
  try {
    const result=await codex.run(f.job);
    assert.equal(result.repository_revision,f.revision);
    assert.equal(result.draft.sections[1].parent,'feature');
    assert.equal(result.manifest.counts.selected_files,1);
    assert.equal(result.evidence.inspected[0].path,'src/sample.js');
    assert.equal(result.evidence.runtime_reproduced,false);
  } finally{codex.close();}
});
for(const mode of ['OUTSIDE','UNREAD','PARENT','HANG']) test(`invalid ${mode} generation is refused`,async t=>{
  const f=await fixture(t);const codex=f.createCodex();
  try{await assert.rejects(codex.run({...f.job,prompt:mode}));}finally{codex.close();}
});
test('a saved draft is recoverable and publication cannot spend inference again',async t=>{
  const f=await fixture(t);let runs=0;const output=join(f.repo,'draft.json');
  const config={repository:f.repo,scope:{include:['src']},output,maxSections:4};
  const result=await generateImport(config,()=>{runs++;return f.createCodex();});
  assert.equal(result.status,'ready');assert.equal(runs,1);
  assert.deepEqual(JSON.parse(await readFile(output,'utf8')),result);
  await assert.rejects(generateImport(config,()=>{runs++;return f.createCodex();}),/EEXIST/);
  assert.equal(runs,1);
  const requests=[];
  for(let i=0;i<2;i++) {
    const published=await publishImport({artifact:result,url:'http://127.0.0.1:3000',token:'test-token',mindmap:'mm-test'},async(url,req)=>{
      requests.push(JSON.parse(req.body));assert.equal(url.pathname,'/v1/mindmaps/mm-test/codebase-import');
      assert.equal(req.redirect,'error');return {ok:true,json:async()=>({root:'mn-root',reviewed:false})};
    });
    assert.equal(published.reviewed,false);
  }
  assert.deepEqual(requests[0],requests[1]);assert.equal(runs,1);
});
test('preflight failures start no model and failed generation is not publishable',async t=>{
  const f=await fixture(t);let runs=0;
  await assert.rejects(generateImport({repository:f.repo,scope:{include:['missing']},output:join(f.repo,'absent.json')},()=>{runs++;return f.createCodex();}));
  assert.equal(runs,0);
  const output=join(f.repo,'failed.json');
  await assert.rejects(generateImport({repository:f.repo,scope:{include:['src']},output,prompt:'UNREAD'},f.createCodex));
  const artifact=JSON.parse(await readFile(output,'utf8'));assert.equal(artifact.status,'failed');
  await assert.rejects(publishImport({artifact,url:'http://127.0.0.1',token:'x',mindmap:'mm-test'}),/ready draft/);
});

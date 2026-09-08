import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, cpSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { execFileSync } from 'node:child_process'

test('builds web before Rust, caches npm work, repairs projected stale assets',()=>{
  const root=mkdtempSync(join(tmpdir(),'takomo-build-contract-'))
  try {
    for (const p of ['scripts','web/src','web/dist/assets','bin']) mkdirSync(join(root,p),{recursive:true})
    cpSync(new URL('./backlot-build.mjs',import.meta.url),join(root,'scripts/backlot-build.mjs'))
    writeFileSync(join(root,'web/package-lock.json'),'{}')
    writeFileSync(join(root,'web/src/app.ts'),'version one')
    writeFileSync(join(root,'web/dist/index.html'),'index')
    writeFileSync(join(root,'web/dist/assets/app.js'),'committed-old')
    writeFileSync(join(root,'bin/npm'),'#!/bin/sh\nprintf "npm %s\\n" "$*" >> "$TEST_LOG"\nif [ "$1" = ci ]; then mkdir -p node_modules; else cp src/app.ts dist/assets/app.js; fi\n',{mode:0o755})
    writeFileSync(join(root,'bin/cargo'),'#!/bin/sh\nprintf "cargo\\n" >> "$TEST_LOG"\n',{mode:0o755})
    const log=join(root,'calls')
    const run=()=>execFileSync(process.execPath,[join(root,'scripts/backlot-build.mjs')],{env:{...process.env,PATH:join(root,'bin')+':'+process.env.PATH,TEST_LOG:log}})
    const calls=()=>readFileSync(log,'utf8').trim().split('\n')
    run(); assert.deepEqual(calls(),['npm ci --no-audit --no-fund','npm run build','cargo'])
    writeFileSync(log,'');run();assert.deepEqual(calls(),['cargo'])
    writeFileSync(join(root,'web/src/app.ts'),'version two');writeFileSync(log,'');run();assert.deepEqual(calls(),['npm run build','cargo']);assert.equal(readFileSync(join(root,'web/dist/assets/app.js'),'utf8'),'version two')
    writeFileSync(join(root,'web/dist/assets/app.js'),'committed-old');writeFileSync(log,'');run();assert.deepEqual(calls(),['npm run build','cargo'])
    writeFileSync(join(root,'web/package-lock.json'),'{"changed":true}');writeFileSync(log,'');run();assert.deepEqual(calls(),['npm ci --no-audit --no-fund','npm run build','cargo'])
  } finally {rmSync(root,{recursive:true,force:true})}
})

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const cli=fileURLToPath(new URL('../takomo',import.meta.url));
test('lane CLI preserves literal content, drafts without dispatch, and sends only explicitly',async()=>{
  const requests=[];
  const server=createServer(async(req,res)=>{let body='';for await(const c of req)body+=c;requests.push({method:req.method,path:req.url,body:body?JSON.parse(body):null});res.setHeader('Content-Type','application/json');res.end(JSON.stringify({id:'wl-test',status:'draft'}));});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const run=(args,input='')=>new Promise((resolve,reject)=>{
    const p=spawn('bash',[cli,...args],{env:{...process.env,TAKOMO_URL:`http://127.0.0.1:${server.address().port}`,TAKOMO_TOKEN:'local-test-only',TAKOMO_PROJECT:'demo'}});
    let stderr='';p.stderr.on('data',x=>stderr+=x);p.stdout.resume();p.on('error',reject);p.on('exit',code=>code===0?resolve():reject(Error(stderr)));p.stdin.end(input);
  });
  try {
    const title='Literal `commands` and $(expressions)';
    await run(['lane','new',title]);
    assert.deepEqual(requests[0],{method:'POST',path:'/v1/projects/demo/lanes',body:{title}});
    const body={kind:'implementation',provider:'codex',instructions:'Line one\nLine two',ticket_ids:['demo-a']};
    await run(['lane','handoff','wl-test','--file','-'],JSON.stringify(body));
    assert.deepEqual(requests[1],{method:'POST',path:'/v1/lanes/wl-test/handoffs',body});
    assert.equal(requests.length,2);
    await run(['handoff','send','ho-test']);
    assert.equal(requests[2].path,'/v1/handoffs/ho-test/dispatch');
    await assert.rejects(run(['lane','handoff','wl-test','--file','-'],'[]'));
    assert.equal(requests.length,3);
  } finally {await new Promise(resolve=>server.close(resolve));}
});

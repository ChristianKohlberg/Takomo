import { createInterface } from 'node:readline';
const send = value => process.stdout.write(`${JSON.stringify(value)}\n`);
const config = {};
for (let i = 0; i < process.argv.length; i++) if (process.argv[i] === '-c') {
  const [path, raw] = process.argv[++i].split(/=(.*)/s); let target = config;
  const keys = path.split('.'); for (const key of keys.slice(0,-1)) target = target[key] ??= {};
  target[keys.at(-1)] = JSON.parse(raw);
}
let mode = '';
const checkout = process.argv.includes('--checkout-fixture');
const sourcePath = checkout ? 'examples/extraction-fixture/checkout.mjs' : 'src/sample.js';
const finish = () => {
  const draft = checkout ? {
    title: 'Checkout sample (simulated draft)', summary: 'Deterministic test output, not a model quality evaluation.',
    gaps: ['Payments, persistence and authentication are not implemented in this sample.'],
    sections: [
      { key: 'quote', parent: null, title: 'Order quotes', notes: 'Orders require items with integer quantities from 1 to 20 and nonnegative integer prices in cents.', sources: [{ path: sourcePath, start_line: 2, end_line: 12 }] },
      { key: 'pricing', parent: 'quote', title: 'Discounts and shipping', notes: 'Members receive a 10% discount rounded down to whole cents. Shipping costs 500 cents unless the discounted subtotal reaches 5,000 cents.', sources: [{ path: sourcePath, start_line: 10, end_line: 12 }] },
      { key: 'cancel', parent: null, title: 'Cancellation', notes: 'A customer can cancel their own pending order. An administrator can cancel any pending order.', sources: [{ path: sourcePath, start_line: 15, end_line: 19 }] },
    ],
  } : { title:'Fixture behavior',summary:'Small codebase draft',gaps:['Outside scope not inspected.'],sections:[
    {key:'feature',parent:null,title:'Feature',notes:'Enabled flag is true.',sources:[{path:mode.includes('OUTSIDE')?'outside.js':'src/sample.js',start_line:1,end_line:1}]},
    {key:'detail',parent:mode.includes('PARENT')?'missing':'feature',title:'Detail',notes:'Returns true.',sources:[{path:'src/sample.js',start_line:1,end_line:1}]}] };
  send({ method:'turn/completed',params:{threadId:'import-thread',turn:{id:'import-turn',status:'completed',items:[{id:'answer',type:'agentMessage',phase:'final_answer',text:JSON.stringify(draft)}]}}});
};
createInterface({input:process.stdin}).on('line',line=>{
  const request=JSON.parse(line); const reply=result=>send({id:request.id,result});
  if (request.id===100 && request.result) return finish();
  if (!request.method || request.id===undefined) return;
  if(request.method==='initialize') { if(!request.params.capabilities?.experimentalApi) process.exit(2); return reply({}); }
  if(request.method==='config/read') return reply({config});
  if(request.method==='thread/start') {
    if(request.params.config.features.shell_tool!==false || !request.params.dynamicTools.some(t=>t.name==='repository_read')) process.exit(2);
    return reply({thread:{id:'import-thread'}});
  }
  if(request.method==='turn/start') {
    if(!request.params.outputSchema?.properties?.sections) process.exit(2);
    mode=request.params.input[0].text;reply({turn:{id:'import-turn'}});
    if(mode.includes('HANG'))return;
    if(mode.includes('UNREAD'))return finish();
    send({id:100,method:'item/tool/call',params:{threadId:'import-thread',turnId:'import-turn',tool:'repository_read',arguments:{path:sourcePath}}});
  }
});

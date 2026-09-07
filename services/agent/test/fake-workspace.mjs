import { createInterface } from 'node:readline';
const config = {};
for (let i = 0; i < process.argv.length; i++) {
  if (process.argv[i] !== '-c') continue;
  const [path, raw] = process.argv[++i].split(/=(.*)/s);
  const keys = path.split('.');
  let target = config;
  for (const key of keys.slice(0, -1)) target = target[key] ??= {};
  target[keys.at(-1)] = JSON.parse(raw);
}
const send = message => process.stdout.write(`${JSON.stringify(message)}\n`);
let prompt = '';
const requestTool = (id, tool, args) => send({ id, method: 'item/tool/call', params: { threadId: 'workspace-thread', turnId: 'workspace-turn', tool, arguments: args } });
const finish = text => send({ method: 'turn/completed', params: { threadId: 'workspace-thread', turn: { id: 'workspace-turn', status: 'completed', items: [{ id: 'answer', type: 'agentMessage', phase: 'final_answer', text }] } } });
createInterface({ input: process.stdin }).on('line', line => {
  const request = JSON.parse(line);
  const reply = result => send({ id: request.id, result });
  if (request.id >= 100 && request.result) {
    if (request.id === 105) return finish(request.result.success ? 'UNEXPECTED TOOL SUCCESS' : request.result.contentItems[0].text);
    if (!request.result.success) return process.exit(3);
    if (request.id === 100) return requestTool(101, 'document_search', { query: 'Invoices' });
    if (request.id === 101) return requestTool(102, 'document_read', { section_id: 'a' });
    return finish(`${request.result.contentItems[0].text}${prompt.includes('legacy-marker') ? ' legacy-marker' : ''}`);
  }
  if (!request.method || request.id === undefined) return;
  if (request.method === 'initialize') { if (!request.params.capabilities?.experimentalApi) process.exit(2); return reply({}); }
  if (request.method === 'config/read') return reply({ config });
  if (request.method === 'thread/read') return reply({ thread: { id: request.params.threadId, turns: [{ status: 'completed', items: [{ type: 'userMessage', content: [{ type: 'text', text: 'legacy-marker' }] }] }] } });
  if (request.method === 'thread/start' || request.method === 'thread/resume') {
    if (!request.params.config.features.code_mode_host || request.params.config.features.shell_tool) return process.exit(2);
    if (request.method === 'thread/start' && request.params.dynamicTools.map(tool => tool.name).sort().join(',') !== 'document_outline,document_read,document_search') return process.exit(2);
    if (request.method === 'thread/resume' && request.params.dynamicTools) return process.exit(2);
    return reply({ thread: { id: 'workspace-thread' } });
  }
  if (request.method === 'turn/start') {
    prompt = request.params.input[0].text;
    reply({ turn: { id: 'workspace-turn' } });
    if (prompt.includes('UNSUPPORTED_REPOSITORY_TOOL')) return requestTool(105, 'repository_read', { path: '/etc/passwd' });
    return requestTool(100, 'document_outline', {});
  }
});

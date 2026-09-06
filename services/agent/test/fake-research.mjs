import { restrictions } from '../codex.mjs';
import { createInterface } from 'node:readline';
const send = message => process.stdout.write(`${JSON.stringify(message)}\n`);
let mode;
const finish = (text, status = 'completed') => send({ method: 'turn/completed', params: { threadId: 'research-thread', turn: { id: 'research-turn', status, items: [{ id: 'answer', type: 'agentMessage', phase: 'final_answer', text }] } } });
createInterface({ input: process.stdin }).on('line', line => {
  const request = JSON.parse(line);
  const reply = result => send({ id: request.id, result });
  if (request.id === 100 && request.result) {
    if (!request.result.success) return finish('Tool failed', 'failed');
    if (mode.includes('READ_HANG')) return;
    return finish(`Code inspection: sample.js:1 ${request.result.contentItems[0].text}`);
  }
  if (!request.method || request.id === undefined) return;
  if (request.method === 'initialize') { if (!request.params.capabilities?.experimentalApi) process.exit(2); return reply({}); }
  if (request.method === 'config/read') return reply({ config: restrictions });
  if (request.method === 'thread/start') {
    if (request.params.config.features.shell_tool !== false || request.params.dynamicTools.length < 2) process.exit(2);
    return reply({ thread: { id: 'research-thread' } });
  }
  if (request.method === 'turn/start') {
    mode = request.params.input[0].text;
    reply({ turn: { id: 'research-turn' } });
    if ((mode.includes('HANG') && !mode.includes('READ_HANG')) || mode.includes('STEER')) return;
    return send({ id: 100, method: 'item/tool/call', params: { threadId: 'research-thread', turnId: 'research-turn', tool: 'repository_read', arguments: { path: 'sample.js' } } });
  }
  if (request.method === 'turn/steer') {
    if (request.params.expectedTurnId !== 'research-turn') process.exit(3);
    reply({ turnId: 'research-turn' });
    return finish(`Steered: ${request.params.input[0].text}`);
  }
  if (request.method === 'turn/interrupt') { reply({}); return finish('', 'interrupted'); }
});

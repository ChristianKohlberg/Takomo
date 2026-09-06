import { restrictions } from '../codex.mjs';
import { createInterface } from 'node:readline';
const send = message => process.stdout.write(`${JSON.stringify(message)}\n`);
createInterface({ input: process.stdin }).on('line', line => {
  const request = JSON.parse(line);
  if (!request.method || request.id === undefined) return;
  const reply = result => send({ id: request.id, result });
  if (request.method === 'initialize') return reply({});
  if (request.method === 'config/read') return reply({ config: restrictions });
  if (request.method.startsWith('thread/')) {
    if (request.params.sandbox !== 'read-only' || request.params.config.features.shell_tool !== false) process.exit(2);
    return reply({ thread: { id: request.params.threadId || 'thread-new' } });
  }
  if (request.method === 'turn/start') {
    const threadId = request.params.threadId;
    const text = request.params.input[0].text;
    if (text.includes('EXIT')) return process.exit(1);
    if (text.includes('TOOL')) return send({ id: 1000, method: 'item/commandExecution/requestApproval', params: { threadId } });
    reply({ turn: { id: 'turn-1' } });
    if (text.includes('HANG')) return;
    send({ method: 'item/completed', params: { threadId: 'unrelated-thread', turnId: 'turn-1', item: { id: 'bad', type: 'agentMessage', text: 'WRONG' } } });
    send({ method: 'item/completed', params: { threadId, turnId: 'turn-1', item: { id: 'commentary', type: 'agentMessage', phase: 'commentary', text: 'Working...' } } });
    const item = { id: 'final', type: 'agentMessage', phase: 'final_answer', text: `Which deadline applies? (${threadId})` };
    send({ method: 'item/completed', params: { threadId, turnId: 'turn-1', item } });
    send({ method: 'turn/completed', params: { threadId, turn: { id: 'turn-1', status: text.includes('FAIL') ? 'failed' : 'completed', items: [item], error: { message: 'Provider unavailable' } } } });
  }
});

import { createInterface } from 'node:readline';
function effectiveConfig() {
  const config = {};
  const argv = process.argv;
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] !== '-c') continue;
    const [path, raw] = argv[++i].split(/=(.*)/s);
    let target = config;
    const keys = path.split('.');
    for (const key of keys.slice(0, -1)) target = target[key] ??= {};
    target[keys.at(-1)] = JSON.parse(raw);
  }
  return config;
}
const send = message => process.stdout.write(`${JSON.stringify(message)}\n`);
createInterface({ input: process.stdin }).on('line', line => {
  const request = JSON.parse(line);
  if (!request.method || request.id === undefined) return;
  const reply = result => send({ id: request.id, result });
  if (request.method === 'initialize') return reply({});
  if (request.method === 'config/read') return reply({ config: effectiveConfig() });
  if (request.method.startsWith('thread/')) {
    if (request.params.sandbox !== 'read-only' || request.params.config.features.shell_tool !== false || request.params.config.features.code_mode_host !== false || request.params.dynamicTools) process.exit(2);
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

import { createInterface } from 'node:readline';
const send = message => process.stdout.write(`${JSON.stringify(message)}\n`);
function effectiveConfig() {
  const config = {};
  for (let i = 0; i < process.argv.length; i++) {
    if (process.argv[i] !== '-c') continue;
    const [path, raw] = process.argv[++i].split(/=(.*)/s);
    const keys = path.split('.'); let target = config;
    for (const key of keys.slice(0, -1)) target = target[key] ??= {};
    target[keys.at(-1)] = JSON.parse(raw);
  }
  return config;
}
createInterface({ input: process.stdin }).on('line', line => {
  const request = JSON.parse(line);
  if (!request.method || request.id === undefined) return;
  const reply = result => send({ id: request.id, result });
  if (request.method === 'initialize') return reply({});
  if (request.method === 'config/read') return reply({ config: effectiveConfig() });
  if (request.method === 'thread/start' || request.method === 'thread/resume') {
    const p = request.params;
    if (p.sandbox !== 'read-only' || p.approvalPolicy !== 'never' || p.dynamicTools || p.config.features.code_mode_host !== false || p.config.features.shell_tool !== false || !p.baseInstructions.includes('project lane organizer')) process.exit(2);
    return reply({ thread: { id: p.threadId || 'organizer-thread' } });
  }
  if (request.method === 'turn/interrupt') return reply({});
  if (request.method === 'turn/start') {
    const p = request.params, text = p.input[0].text;
    const schema = p.outputSchema;
    if (p.sandboxPolicy.type !== 'readOnly' || p.sandboxPolicy.networkAccess !== false || schema?.additionalProperties !== false || !schema?.required.includes('groups') || !schema.properties.groups.items.required.includes('readiness')) process.exit(3);
    if (!text.startsWith('PROJECT LANE ORGANIZER SNAPSHOT')) process.exit(4);
    const snapshot = JSON.parse(text.slice(text.indexOf(':\n') + 2, text.lastIndexOf('\n\nUSER MESSAGE:\n')));
    const threadId = p.threadId;
    if (text.includes('UNSUPPORTED_TOOL')) return send({ id: 1000, method: 'item/tool/call', params: { threadId, tool: 'repository_read', arguments: { path: '/etc/passwd' } } });
    reply({ turn: { id: 'organizer-turn' } });
    if (text.includes('HANG')) return;
    const lane = snapshot.lanes[0];
    const proposal = {
      groups: [
        { lane_id: lane.id, title: lane.title, purpose: lane.purpose, context: 'Test the supplied denim seam reference.', readiness: 'ready', reason: 'Material and acceptance criteria are specified.', ticket_ids: [snapshot.tickets[0].id] },
        { lane_id: null, title: 'Fit samples', purpose: 'Resolve sample fit', context: 'Waist tolerances need clarification.', readiness: 'needs_clarification', reason: 'The requested waist tolerance is unspecified.', ticket_ids: [snapshot.tickets[1].id] },
      ],
      unassigned: [{ ticket_id: snapshot.tickets[2].id, reason: `Possible duplicate of ${snapshot.tickets[0].id}; confirm before grouping.` }],
    };
    if (text.includes('UNKNOWN_TICKET')) proposal.groups[0].ticket_ids = ['invented-ticket'];
    if (text.includes('RENAMED_LANE')) proposal.groups[0].title = 'An unauthorized rename';
    if (text.includes('BIG_VALID')) for (const group of proposal.groups) group.context = 'x'.repeat(40_000);
    let message = text.includes('INVALID_JSON') ? '```json\n{}\n```' : JSON.stringify(proposal);
    if (text.includes('OVERSIZED')) message = 'x'.repeat(256_001);
    const item = { id: 'proposal', type: 'agentMessage', phase: 'final_answer', text: message };
    send({ method: 'item/completed', params: { threadId: 'other-organizer', turnId: 'organizer-turn', item: { ...item, text: 'WRONG' } } });
    send({ method: 'item/completed', params: { threadId, turnId: 'organizer-turn', item } });
    send({ method: 'turn/completed', params: { threadId, turn: { id: 'organizer-turn', status: text.includes('FAILED_TURN') ? 'failed' : 'completed', items: [item], error: { message: 'Provider unavailable' } } } });
  }
});

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
const tool = (id, name, args) => send({ id, method: 'item/tool/call', params: { threadId: 'classification-thread', turnId: 'classification-turn', tool: name, arguments: args } });
let mode = '';
createInterface({ input: process.stdin }).on('line', line => {
  const request = JSON.parse(line);
  const reply = result => send({ id: request.id, result });
  if (request.id >= 100 && request.result) {
    if (!request.result.success) return process.exit(3);
    if (request.id === 100) return tool(101, 'document_search', { query: 'Invoice expiration' });
    if (request.id === 101) return tool(102, 'document_read', { section_id: 'invoice' });
    const source = JSON.parse(request.result.contentItems[0].text);
    const candidate = { section_id: source.section_id, version: source.version, quote: 'Invoices expire after 30 days.', rationale: 'The requested deadline behavior is specified here.' };
    const proposal = { candidates: [candidate], ambiguity: null, no_match_reason: null };
    if (mode.includes('NO_MATCH')) { proposal.candidates = []; proposal.no_match_reason = 'The document does not describe the requested behavior.'; }
    if (mode.includes('AMBIGUOUS')) proposal.ambiguity = 'The ticket does not specify whether the deadline is calendar or business days.';
    if (mode.includes('UNKNOWN_SECTION')) candidate.section_id = 'invented';
    if (mode.includes('STALE_VERSION')) candidate.version = '0'.repeat(64);
    if (mode.includes('BAD_QUOTE')) candidate.quote = 'This text does not exist.';
    if (mode.includes('DUPLICATE')) proposal.candidates.push(candidate);
    if (mode.includes('CONFIDENCE')) proposal.confidence = 0.99;
    const text = mode.includes('INVALID_JSON') ? '```json\n{}\n```' : JSON.stringify(proposal);
    return send({ method: 'turn/completed', params: { threadId: 'classification-thread', turn: { id: 'classification-turn', status: 'completed', items: [{ id: 'proposal', type: 'agentMessage', phase: 'final_answer', text }] } } });
  }
  if (!request.method || request.id === undefined) return;
  if (request.method === 'initialize') { if (!request.params.capabilities?.experimentalApi) process.exit(2); return reply({}); }
  if (request.method === 'config/read') return reply({ config });
  if (request.method === 'thread/start') {
    const p = request.params;
    if (!p.config.features.code_mode_host || p.config.features.shell_tool || p.approvalPolicy !== 'never' || p.dynamicTools.map(t => t.name).sort().join(',') !== 'document_outline,document_read,document_search') return process.exit(2);
    if (!p.baseInstructions.includes('ticket-to-document classifier')) return process.exit(2);
    return reply({ thread: { id: 'classification-thread' } });
  }
  if (request.method === 'turn/start') {
    const p = request.params;
    if (!p.outputSchema?.required.includes('candidates') || p.outputSchema.properties.candidates.maxItems !== 3 || p.outputSchema.additionalProperties !== false || p.sandboxPolicy.networkAccess !== false) return process.exit(2);
    mode = p.input[0].text;
    reply({ turn: { id: 'classification-turn' } });
    return tool(100, 'document_outline', {});
  }
});

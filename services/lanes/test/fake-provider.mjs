import { writeFileSync } from 'node:fs';
import { spawn } from 'node:child_process';
const provider = process.argv.find(arg => arg.startsWith('--fake-provider='))?.split('=')[1];
const action = process.argv.find(arg => arg.startsWith('--fake-action='))?.split('=')[1];
let prompt = '';
for await (const chunk of process.stdin) prompt += chunk;
if (process.env.FAKE_RECORD) writeFileSync(process.env.FAKE_RECORD, JSON.stringify({ args: process.argv.slice(2), prompt, env: process.env, cwd: process.cwd() }));
if (action === 'hang') {
  setInterval(() => {}, 1000);
} else if (action === 'descendant') {
  const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'inherit' });
  if (process.env.FAKE_PID) writeFileSync(process.env.FAKE_PID, String(child.pid));
  setInterval(() => {}, 1000);
} else if (action === 'exit') {
  console.error('provider secret diagnostic must never become a saved result'); process.exitCode = 3;
} else if (action === 'invalid') {
  console.log('invalid output');
} else if (provider === 'codex') {
  console.log(JSON.stringify({ type: 'thread.started', thread_id: '11111111-1111-1111-1111-111111111111' }));
  console.log(JSON.stringify({ type: 'item.completed', item: { type: 'agent_message', text: action === 'large' ? 'x'.repeat(65000) : 'Checked the lane.' } }));
  console.log(JSON.stringify({ type: action === 'failed' ? 'turn.failed' : 'turn.completed' }));
} else {
  console.log(JSON.stringify({ type: 'result', result: 'Checked the lane.', session_id: '22222222-2222-2222-2222-222222222222', is_error: action === 'failed' }));
}

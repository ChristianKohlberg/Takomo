import test from 'node:test';
import assert from 'node:assert/strict';
import { CodexConnection } from '../codex-connection.mjs';
function fixture(action = 'login') {
 const calls = []; let signedIn = false;
 const client = { logins: new Map(), async initialize() {}, close() {}, async request(method, params) {
  calls.push({ method, params });
  if (method === 'account/read') return { account: signedIn ? { type: 'chatgpt', email: 'test@example.invalid', planType: 'plus', accessToken: 'never-forward' } : null };
  if (method === 'account/login/start') return { loginId: 'provider-login', verificationUrl: 'https://auth.openai.com/codex/device', userCode: 'ABCD-1234' };
  if (method === 'account/logout') signedIn = false;
  if (method === 'account/rateLimits/read') return { rateLimits: { primary: { usedPercent: 20, resetsAt: 2000000000 } } };
  return {};
 }};
 const server = { action, command_id: 'request-1', expires_at: Date.now()+600000, report: { status: 'unknown' } };
 const reports = [];
 const control = new CodexConnection({ serviceId:'worker', createClient:()=>client, api:async (_path, body)=>{
  reports.push(body);
  if (body.command_id === server.command_id && body.report) { server.report=body.report; if (body.report.status !== 'login_pending') server.action=null; }
  return structuredClone(server);
 }});
 return { control,client,server,calls,reports,signIn:()=>{signedIn=true;client.logins.set('provider-login',true);} };
}
test('device login pauses claims, completes once, forwards only safe account metadata and quotas', async()=>{
 const f=fixture();assert.equal(await f.control.tick(),true);
 assert.equal(f.server.report.device.user_code,'ABCD-1234');
 assert.equal(await f.control.tick(),true);
 assert.equal(f.calls.filter(c=>c.method==='account/login/start').length,1);
 f.signIn();assert.equal(await f.control.tick(),false);
 assert.equal(f.server.report.status,'connected');
 assert.equal(f.server.report.limits.primary.used_percent,20);
 assert.ok(!JSON.stringify(f.reports).includes('never-forward'));
 assert.ok(!JSON.stringify(f.reports).includes('provider-login'));
 assert.ok(f.calls.every(c=>c.method.startsWith('account/')));
});
test('cancel replaces a pending login and never turns into an inference request',async()=>{
 const f=fixture();await f.control.tick();f.server.command_id='cancel-2';f.server.action='cancel';
 assert.equal(await f.control.tick(),true);
 assert.ok(f.calls.some(c=>c.method==='account/login/cancel'));
 assert.equal(f.server.report.status,'disconnected');
});
test('worker restart does not automatically replay an interrupted device login',async()=>{
 const f=fixture();f.server.report.status='login_pending';
 assert.equal(await f.control.tick(),true);
 assert.equal(f.server.report.status,'error');
 assert.ok(!f.calls.some(c=>c.method==='account/login/start'));
});
test('logout clears provider authentication and pauses new work',async()=>{
 const f=fixture('logout');f.signIn();
 assert.equal(await f.control.tick(),true);
 assert.ok(f.calls.some(c=>c.method==='account/logout'));
 assert.equal(f.server.report.status,'disconnected');
});
test('stdio account client retains login notifications received before the RPC response', async()=>{
 const { AccountClient } = await import('../codex-connection.mjs');
 const { fileURLToPath } = await import('node:url');
 const client = new AccountClient({ executable: process.execPath, args: [fileURLToPath(new URL('./fake-account.mjs', import.meta.url))], cwd:'/tmp', home:'/tmp' });
 try {
  await client.initialize();
  const result = await client.request('account/login/start', { type:'chatgptDeviceCode' });
  assert.equal(client.logins.get(result.loginId),true);
 } finally { client.close(); }
});

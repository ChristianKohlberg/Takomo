import { createInterface } from 'node:readline';
const send = value => process.stdout.write(JSON.stringify(value)+'\n');
createInterface({ input: process.stdin }).on('line', line => {
 const request = JSON.parse(line);
 if (request.id === undefined) return;
 if (request.method === 'initialize') return send({ id: request.id, result: {} });
 if (request.method === 'account/login/start' && request.params.type === 'chatgptDeviceCode') {
  send({ method: 'account/login/completed', params: { loginId: 'login-1', success: true } });
  return send({ id: request.id, result: { loginId: 'login-1', verificationUrl: 'https://auth.openai.com/codex/device', userCode: 'CODE-1234' } });
 }
 send({ id: request.id, error: { code: -32601, message: 'Unexpected RPC' } });
});

import { Codex } from './codex.mjs';

// Account RPCs use the same stdio-only process policy as jobs, without starting a model turn.
export class AccountClient extends Codex {
  constructor(options) { super(options); this.logins = new Map(); }
  receive(event) {
    if (event.method === 'account/login/completed') { this.logins.set(event.params.loginId, event.params.success === true); return; }
    super.receive(event);
  }
  async initialize() {
    await this.request('initialize', { clientInfo: { name: 'takomo_account', title: 'Takomo connection settings', version: '0.1.0' } });
    this.send({ method: 'initialized' });
  }
}
const failure = () => ({ status: 'error', error: 'Codex connection request failed. Check the worker and account permissions, then refresh or reconnect.' });
const window = value => value && Number.isInteger(value.usedPercent) && value.usedPercent >= 0 && value.usedPercent <= 100 ? { used_percent: value.usedPercent, resets_at: Number.isSafeInteger(value.resetsAt) && value.resetsAt >= 0 ? value.resetsAt : null } : null;
export class CodexConnection {
  constructor({ api, serviceId, createClient }) {
    this.api = api; this.serviceId = serviceId; this.createClient = createClient;
    this.report = { status: 'unknown' }; this.commandId = null; this.handled = null;
  }
  async client() {
    if (!this.codex || this.codex.failure) {
      this.codex?.close(); this.codex = this.createClient(); await this.codex.initialize();
    }
    return this.codex;
  }
  async refresh() {
    const client = await this.client();
    const { account } = await client.request('account/read', { refreshToken: true });
    if (!account) { this.report = { status: 'disconnected' }; return; }
    const text = (s, max) => typeof s === 'string' && s.length <= max ? s : null;
    this.report = { status: 'connected', account: { auth_mode: account.type === 'chatgpt' ? 'chatgpt' : account.type === 'apiKey' ? 'apiKey' : 'other', email: text(account.email, 254), plan: text(account.planType, 80) } };
    if (account.type === 'chatgpt') {
      try { const { rateLimits } = await client.request('account/rateLimits/read', {}); this.report.limits = { primary: window(rateLimits?.primary), secondary: window(rateLimits?.secondary) }; }
      catch { /* Quota unavailable does not imply the account is disconnected. */ }
    }
  }
  async cancel() {
    if (this.loginId && this.codex && !this.codex.failure) await this.codex.request('account/login/cancel', { loginId: this.loginId });
    this.loginId = null;
  }
  async poll(busy = false) {
    return this.api('/v1/agent-services/codex/poll', { service_id: this.serviceId, busy, ...(busy ? {} : { command_id: this.commandId, report: this.report }) });
  }
  async tick() {
    if (!this.initialized) { this.initialized = true; try { await this.refresh(); } catch { this.report = failure(); } }
    if (this.loginId) {
      const completed = this.codex?.logins.get(this.loginId);
      if (completed !== undefined) {
        this.codex.logins.delete(this.loginId); this.loginId = null;
        try { if (!completed) throw new Error(); await this.refresh(); } catch { this.report = failure(); }
      } else if (Date.now() >= this.expiresAt || this.codex?.failure) {
        try { await this.cancel(); } catch { this.loginId = null; }
        this.report = { status: 'error', error: 'Login expired or the worker connection was interrupted. Start a new login.' };
      }
    }
    let connection = await this.poll();
    if (!connection.action) {
      // A late login completion cannot resurrect an expired or cancelled server request.
      if (this.loginId) { await this.cancel(); await this.refresh(); }
      this.commandId = null;
      return this.report.status !== 'connected';
    }
    if (connection.command_id !== this.handled) {
      this.handled = connection.command_id; this.commandId = connection.command_id;
      try {
        if (connection.action === 'login' && connection.report.status === 'login_pending') throw new Error('Restarted pending login');
        await this.cancel();
        const client = await this.client();
        client.logins.clear();
        if (connection.action === 'login') {
          const result = await client.request('account/login/start', { type: 'chatgptDeviceCode' });
          if (result.verificationUrl !== 'https://auth.openai.com/codex/device' || typeof result.userCode !== 'string' || !/^[A-Za-z0-9-]{1,32}$/.test(result.userCode) || typeof result.loginId !== 'string') throw new Error('Invalid login response');
          this.loginId = result.loginId; this.expiresAt = connection.expires_at;
          this.report = { status: 'login_pending', device: { verification_url: result.verificationUrl, user_code: result.userCode } };
        } else {
          if (connection.action === 'logout') await client.request('account/logout', {});
          await this.refresh();
        }
      } catch { this.codex?.close(); this.loginId = null; this.report = failure(); }
      connection = await this.poll();
    }
    return !!connection.action || this.report.status !== 'connected';
  }
  async busy(value) { try { await this.poll(value); } catch { /* Job lease remains authoritative. */ } }
  close() { this.codex?.close(); }
}

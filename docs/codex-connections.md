# Codex authentication in Takomo

Settings → AI connections lets unrestricted human administrators manage the Codex
account used by a registered worker. This MVP uses an operator-managed account
per worker; its permitted projects remain the worker token's project restrictions.
It does not grant additional project permissions or implement personal accounts
per Takomo user. Account switching affects future jobs for all listed projects.

Deploy the updated server first. Install the updated `services/agent/*.mjs` files
on the worker and enable `TAKOMO_CODEX_CONNECTIONS=1` in its service environment.
Restart the worker while idle, preserving its existing state directory and token.
The setting is opt-in; workers without it retain the existing local-login flow.
No change to the Codex CLI is needed for the verified 0.153.4 account RPC schema.

The worker registers via authenticated outbound HTTPS. Settings lists its last
contact, busy state, permitted projects, account and reported quota windows.
Click Connect ChatGPT / Codex, open the OpenAI verification link and enter the
one-time code. Device-code authentication may need enabling in ChatGPT security
settings or workspace permissions. Takomo never receives the password, access
or refresh token. Codex stores and refreshes credentials in the worker's dedicated
Codex home. Account details, temporary device codes and quota metadata are visible
only to unrestricted administrators, with no-store responses.

Authentication uses `account/login/start` with `chatgptDeviceCode`, completion
notifications, `account/read`, `account/rateLimits/read`, `account/login/cancel`
and `account/logout`. It starts no model turn and uses stdio, with no callback
listener, inbound worker port or tunnel. The worker relays a fixed set of commands;
HTTP clients cannot submit arbitrary app-server RPCs. Quota windows are account
limits; per-run token usage remains in the Agent queue and is not a currency bill.

Changes queue until the worker is idle. A pending login or disconnected account
pauses new job claims. Disconnect signs out this worker's Codex home; it does not
revoke unrelated ChatGPT sessions. Cancel stops a pending local device login.
Requests expire after ten minutes, stale responses cannot complete a replacement
request, and an interrupted login must be explicitly restarted. Quota failures
leave a valid account connected and show no reported limits.

Worker identities are bound to the authenticating Takomo token plus service ID.
Rotating that token registers a new connection entry; the old entry remains stale
and cannot be driven by the new credential. There is a 100-worker registration
limit. Keep one service process per state directory.

Validation uses fake account RPCs and real HTTP/SQLite tests. A live login requires
an administrator to approve the device code on OpenAI; it is not run automatically.
For rollout, verify one account refresh and device login after deployment. Before
rollback, disable connection management and cancel pending login requests, then
restore the previous worker before the old server. Preserve the credential home;
the additive database table can remain. Disabling the feature restores ordinary
local authentication and does not itself sign out an account.

Official protocol: https://learn.chatgpt.com/docs/app-server
Authentication: https://learn.chatgpt.com/docs/auth

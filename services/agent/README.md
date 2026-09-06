# Takomo agent service

A standalone, single-job worker for read-only section conversations. It claims jobs from Takomo, runs a Codex App Server turn over stdio, and delivers the completed response. The same process can run beside Takomo, on a developer machine, or on another server; it only needs an outbound connection to Takomo and Codex's provider.

## Start

Requires Node.js 22+ and Codex CLI. The protocol and restrictive configuration were checked with Codex **0.153.4**. No npm dependencies are needed.

1. Choose a **dedicated persistent service state directory**. It contains the service ID, Codex authentication and conversation state, and an empty workspace. Do not point it at an existing repository or your normal Codex home.
2. Authenticate Codex into that dedicated home using its supported login flow:

   ```sh
   mkdir -p "$HOME/.takomo-agent/codex"
   CODEX_HOME="$HOME/.takomo-agent/codex" codex login
   ```

   For a headless host, use the login method supported by your Codex installation (`codex login --help`). Authentication is explicit: this service does not copy credentials or inherit provider API keys from the launching shell. Retain the Codex state directory across restarts. Protect it as credentials and conversation content.
3. On the Takomo host, mint a token restricted to the intended project (use your actual database path and project slug):

   ```sh
   takomo --db /path/to/takomo.db token create --actor agent:section-review --scopes agent:run --projects my-project
   ```

   The token has queue access only. It cannot edit specifications, tickets, or tests. Then start the service:

   ```sh
   export TAKOMO_URL=http://127.0.0.1:3000
   export TAKOMO_AGENT_TOKEN='<agent:run token>'
   node services/agent/service.mjs
   ```

Use `--once` to claim at most one immediately available job and exit after its result is delivered. No queued job is a successful no-op. Model or Codex configuration failures are delivered as failed jobs and exit successfully after delivery. Startup configuration and unrecoverable queue/result transport failures exit nonzero.

| Setting | Default |
| --- | --- |
| `TAKOMO_URL` | Required; HTTPS except on loopback |
| `TAKOMO_AGENT_TOKEN` | Required; token with `agent:run` scope |
| `TAKOMO_AGENT_STATE_DIR` | `~/.takomo-agent` |
| `TAKOMO_AGENT_SERVICE_ID` | Random stable ID saved in the state directory |
| `TAKOMO_CODEX_BIN` | `codex` on PATH |

The service's persisted ID and Codex state belong together. Run only one service process against a state directory. Changing the identity/home does not migrate existing conversations. A system supervisor can start this command, provide the environment, and restart it after a crash; no inbound worker port is needed.

## Behavior and boundaries

- Jobs are claimed sequentially with 25-second long polling. Idle/transient connection failures use bounded backoff. Invalid worker credentials stop the process.
- Session/thread and turn IDs are sent to Takomo as soon as they exist, before the answer. Follow-ups resume the saved thread. Only final user-facing assistant text is returned; commentary and reasoning are excluded.
- A heartbeat renews the job lease every 15 seconds. Losing a heartbeat stops Codex, preventing the worker from continuing after losing ownership. Takomo marks expired jobs failed. Turns are never automatically rerun; result delivery alone may be retried, using the same attempt ID and payload.
- Codex starts in an empty workspace with a dedicated HOME/CODEX_HOME and a small environment allowlist. The Takomo token and parent process secrets are not passed to Codex. Read-only sandbox policy and disabled network access apply to each turn. Shell execution, apps, plugins, hooks, browser/computer tools, images, multi-agent tools, and web search are disabled. Before starting a thread, effective configuration is checked: inherited MCP servers/plugins, enabled restricted features, relaxed permissions, and custom instruction files/notify hooks fail closed. Unsupported tool/approval requests fail the turn.
- No document API, edit tool, or test creation tool is exposed. Section text is supplied as review material. The worker has a five-minute turn timeout and a 64,000-byte response limit, and SIGINT/SIGTERM stop the active Codex process. An interrupted job is resolved by lease expiry.
- This MVP returns the response after completion; it does not stream text, expose approval dialogs, migrate sessions, retry failed turns, or generate code.

## Check

```sh
node --test services/agent/test/*.test.mjs
```

The fake JSON-RPC process tests thread start/resume, early IDs, final-answer filtering/deduplication, provider failure, process death, unsupported approvals, timeouts, result retry without model reexecution, lease-loss handling, and rejection of inherited tool configurations. To check the real integration, authenticate the dedicated home, queue a section grill in Takomo, run `--once`, then submit a follow-up and run it again. Reload Takomo to verify persisted messages. Stop a running service to verify lease expiry displays a failed job.

Protocol references: [Codex App Server](https://learn.chatgpt.com/docs/app-server), [Codex configuration](https://learn.chatgpt.com/docs/config-file/config-reference). For a CLI upgrade, regenerate its schema with `codex app-server generate-json-schema --out /tmp/codex-schema` and rerun these checks.

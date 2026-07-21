# Takomo — Claude Code plugin

Talk to a hosted [Takomo](https://github.com/ChristianKohlberg/Takomo)
from Claude Code. This plugin bundles:

- **The `takomo` skill** — teaches the agent to use the central store as the
  single source of truth for work (find, claim, progress tickets through their
  enforced workflow, attach evidence) instead of an in-session todo list.
- **A remote MCP server** — a Model Context Protocol server declaration pointing
  at your Takomo's `/mcp` endpoint over HTTP (read from `${TAKOMO_MCP_URL}`), so
  the store's tools are available directly in the session.

## Install (two lines)

This repository doubles as the plugin marketplace. From inside Claude Code:

```
/plugin marketplace add ChristianKohlberg/Takomo
/plugin install takomo
```

## Supply your host + token

Nothing is baked into the plugin — it reads both from the environment, so set
them before starting Claude Code (or in your shell profile):

```sh
export TAKOMO_MCP_URL="https://<your-host>/mcp"   # the MCP endpoint — note /mcp, not /v1
export TAKOMO_TOKEN="tk_<your read/write token>"
```

- **`TAKOMO_MCP_URL`** is the MCP endpoint, served at `/mcp`. This is **not** the
  same as the CLI's `TAKOMO_URL`, which points at the REST base `/v1`. Since Takomo
  is self-hosted, the plugin can't guess your host — if `TAKOMO_MCP_URL` is unset the
  server has no host to reach and shows up as *failed to connect*.
- **`TAKOMO_TOKEN`** is a Takomo bearer token — **no token is stored in this repo**.
  Mint a scoped one with an admin token via `takomo token create`, or the
  `POST /v1/tokens` HTTP endpoint — see the
  [auth model](https://github.com/ChristianKohlberg/Takomo#auth-model). The same
  variable is used by the `takomo` CLI, so a repo onboarded with `takomo init`
  already has one.

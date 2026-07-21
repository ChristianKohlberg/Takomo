# Takomo — Claude Code plugin

Talk to a hosted [Takomo](https://github.com/ChristianKohlberg/Takomo)
from Claude Code. This plugin bundles:

- **The `takomo` skill** — teaches the agent to use the central store as the
  single source of truth for work (find, claim, progress tickets through their
  enforced workflow, attach evidence) instead of an in-session todo list.
- **A remote MCP server** — a Model Context Protocol server declaration pointing
  at the hosted endpoint `https://your-takomo-host.onrender.com/mcp` over HTTP, so
  the store's tools are available directly in the session.

## Install (two lines)

This repository doubles as the plugin marketplace. From inside Claude Code:

```
/plugin marketplace add ChristianKohlberg/Takomo
/plugin install takomo
```

## Supply your token

The MCP server authenticates with a Takomo bearer token, read from the
`TAKOMO_TOKEN` environment variable — **no token is stored in this repo**.
Set it in your environment before starting Claude Code (or in your shell
profile):

```sh
export TAKOMO_TOKEN="tk_<your read/write token>"
```

Mint a scoped token with an admin token via `takomo token create`, or the
`POST /v1/tokens` HTTP endpoint — see the
[auth model](https://github.com/ChristianKohlberg/Takomo#auth-model). The
same variable is used by the `takomo` CLI, so a repo onboarded with `takomo init`
already has one.

> The MCP `url` points at the community-hosted store. To target your own
> deployment, install from a fork with the URL changed in
> `plugins/takomo/.mcp.json`.

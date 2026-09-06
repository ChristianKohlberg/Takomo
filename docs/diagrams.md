# Diagrams and wireframes

Document code blocks support **Mermaid**, **PlantUML**, and **D2** through a private
[Kroki](https://kroki.io/) rendering service. **Wireframe** inserts a PlantUML Salt
template. These blocks use the same Code / View controls, sizing and expanded
preview; they are not separate document formats.

The source and language remain in the existing collaborative code block. Editing,
undo, history and agent proposals follow the document's existing rules. Generated
SVG is a preview, not another editable copy. The browser displays it as an image,
never as inline application HTML. Readers can inspect the source and expand the
preview but cannot edit the document.

Rendering requires a connection to Takomo. While editing or after a failed render,
the last successful preview stays visible and is marked outdated. Invalid source
is retained so it can be corrected. Existing Mermaid blocks use Kroki too; deploy
the service and configure Takomo together when upgrading. Without configuration,
the source remains available and the preview explains that rendering is unavailable.

## Run the service on the same machine

Install Docker with Compose, then from the repository root:

```sh
docker compose -f deploy/kroki.compose.yaml up -d
```

The configuration pins Kroki and its Mermaid companion to immutable images.
PlantUML and D2 are included in the main image. Only the gateway is exposed, on
`127.0.0.1:18080`; the companion is on an internal Docker network. No Takomo token
or database is mounted into either container.

Set these variables in the environment of the **Takomo server process**, then
restart that process through its normal service manager:

```sh
TAKOMO_KROKI_URL=http://127.0.0.1:18080
TAKOMO_KROKI_VERSION=0.32.1-secure-v1
```

For a local Backlot session, set both variables in the server service's `env`
configuration in a local preview manifest. Backlot services inherit the daemon's
environment, so exporting variables in an already-running daemon's client shell
does not configure them. For a deployed service, use its service manager or hosting settings;
setting them in a browser or Codex worker does not configure Takomo.

Check the renderer independently with a non-sensitive sample:

```sh
curl --fail-with-body http://127.0.0.1:18080/plantuml/svg \
  -H 'Content-Type: text/plain' \
  --data-binary @- > /tmp/takomo-wireframe.svg <<'PLANTUML'
@startsalt
{ Codex integration | [Connect worker] }
@endsalt
PLANTUML
```

Then open a specification in Takomo, insert **Wireframe**, and switch to **View**.
Also check a Mermaid block and D2 block. The browser's rendering request goes to
Takomo's `/v1/diagrams/render`, not directly to port 18080.

## Run beside a container or on another server

When Takomo is a container on the same Docker network, use the service address
`http://kroki:8000` and connect Takomo to the gateway network. Loopback inside a
container refers to that container, not its host.

For a remote host, use a private network/VPN or authenticated infrastructure proxy
and configure its reachable URL on Takomo. The supplied loopback binding deliberately
does not expose a public rendering endpoint. Kroki does not understand Takomo
tokens; keep it reachable only by the Takomo server. For Render, run the gateway
and companion as private services on the same private network as Takomo and set
the companion host accordingly. The existing `render.yaml` does not provision them.

Kroki runs in `SECURE` mode with PlantUML's `SANDBOX` profile. File and URL includes
are not supported. **D2 is not covered by Kroki's safe mode:** the supplied service
sets `D2_BUNDLE=false` to stop D2 fetching external images and icons. This setting
is also required on a remotely configured Kroki service. Takomo accepts an
intentionally restricted D2 subset without the `@` character, rejecting imports
before they reach the renderer (including an `@` inside a label or comment).
Keep D2 diagrams self-contained; external image/icon assets are unsupported.
Do not relax these settings for user-authored diagrams. The
gateway and companion have separate memory/CPU limits and bounded rendering times.
See the [upstream configuration reference](https://docs.kroki.io/kroki/setup/configuration/)
for the meaning of those settings.

## Limits, caching and upgrades

Takomo requires a normal bearer token with read access to the request's project.
The allowlist is Mermaid, PlantUML and D2; a request cannot choose an upstream URL
or enable another engine. Rendering is a POST and follows the existing per-token
REST request budget, even though it does not mutate document content.

Source is capped at 50 KB and each SVG at 2 MiB. Takomo allows at most four
concurrent upstream renders and bounds each request to ten seconds. Only successful
renders enter the bounded in-memory cache. Permission checks run before cache
lookups. A restart clears that cache; it is not a permanent exported artifact.

Set `TAKOMO_KROKI_VERSION` to identify the renderer images and configuration.
Change it when upgrading engines, fonts or rendering options, and restart Takomo.
The cache is scoped to the configured renderer URL and keyed by version, engine
and source. Identical diagrams can reuse output across projects after access checks.
Pin both container images together and check representative existing diagrams
before rollout; upstream syntax and layout can change between engine releases.

If rendering fails, check Takomo's configuration, gateway connectivity and
`docker compose -f deploy/kroki.compose.yaml logs --tail 100`. Syntax errors are
corrected in Code; unavailable or busy services can be retried in View. Avoid
putting private document source in shared logs or public rendering services.

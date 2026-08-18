# Mindmaps: thinking out loud, before it is anything

Takomo has a home for work (tickets), for an idea being nurtured (initiatives),
and for verification (checks). It had nothing for the ten minutes *before* any of
those — the conversation where a project idea fans out into API, integrations,
workflows, ideas, six words a thought, and a branch splits in two the moment it
turns out to be two thoughts.

A **mindmap** is that surface: a project-scoped tree you grow at conversation
speed, whose branches can graduate into epics and initiatives, and which then
keeps the link — so a map that mattered stays a way to navigate what it became.

## What it deliberately is not

A brainstorming method, nothing more. No workflow, no claim, no lease, no ready
queue, no assignment, no comments, no attachments, no expiry.

Two rules follow, and they shape everything:

**Deleting one is ordinary.** An initiative earns the right to be nurtured; a
mindmap earns the right to be thrown away. `takomo mindmap rm` cascades its nodes
and touches nothing its branches became — those graduated and are work in their
own right. That is what makes it safe to start a map early, which is the only
time a brainstorm is worth anything.

**A node is capped at 280 characters**, and that is the method rather than a
limitation. A brainstorm whose nodes grow into paragraphs has quietly become a
document, and the thing that made it valuable — reading a whole branch at a
glance — is gone by the time anyone notices. So the refusal names the way out:

```
That node is 300 characters and the cap is 280. A mindmap node is a sentence or
two — that brevity is what makes a branch readable at a glance.
remedy: Shorten it, split it into two nodes, or promote the branch to an
initiative where the long form belongs.
```

## Growing one

```sh
takomo mindmap new "Payments rebuild"          # the title is the root
takomo mindmap add mm-1cslpg34 "API"
takomo mindmap add mm-1cslpg34 "versioning: v1 forever, or dated?" --parent mn-hr9hcrgp
takomo mindmap show mm-1cslpg34
```

```
# Payments rebuild   (mm-1cslpg34, open)
- API   [mn-hr9hcrgp]
  - versioning: v1 forever, or dated?   [mn-54tvdp1u]
  - idempotent retries on capture       [mn-jj6tyi3x]
- integrations   [mn-c3gftogt]
- workflows      [mn-r4bk7rc9]
- ideas          [mn-rrnen6n5]
```

The node id sits beside each line because the next thing anyone does is hang a
thought under one of them, and that takes an id.

### With an agent, which is the other half

Brainstorming with Claude means the agent adds a whole branch while you talk, so
the node route takes a **batch** and `takomo_mindmap_grow` is built around it:

```json
{ "id": "mm-1cslpg34", "nodes": [
  { "text": "API" }, { "text": "integrations" }, { "text": "workflows" }
] }
```

Fifty nodes per call, and the batch lands whole or not at all — half a branch
arriving would leave a map nobody asked for and no way to tell which half.
`takomo_mindmap_show` reads it back as the indented text above, which is the
cheapest shape for a model to reason about, with the ids alongside.

## What a branch becomes

```sh
takomo mindmap promote mm-1cslpg34 mn-hr9hcrgp --to epic
takomo mindmap promote mm-1cslpg34 mn-c3gftogt --to initiative
```

| target | makes | for |
|---|---|---|
| `epic` | an epic titled from the node, with its **direct children** as tickets under it | the fastest path from "we talked about it" to work in the queue |
| `initiative` | an initiative titled from the node, with the subtree as its first **entry** | a direction that needs nurturing before it is work |

Only direct children become tickets. A deeper subtree would arrive as a flat pile
whose shape nobody could recover — and the map keeps that shape for whoever wants
it. The initiative gets the subtree as an *entry* rather than a summary because an
entry carries provenance (`source: mindmap:mm-…`), and where an idea came from is
exactly what a collection is supposed to remember.

**The node stays**, carrying what it became:

```
- API   [mn-hr9hcrgp]  -> epic demo-ziuq
- integrations   [mn-c3gftogt]  -> initiative ini-dx161vhm
```

Promotion is not a move. The map is the record of how the thinking got there, and
a branch that vanished the moment it mattered would make the map worthless as the
thing you read afterwards. This is also the answer to "can it stay longer as a
navigation tool": a map that produced work becomes a picture of that work.

Promoting the same branch twice is refused — it would make a second epic from one
thought, indistinguishable from the first.

## Caps, and why each one exists

| cap | value | why |
|---|---|---|
| node text | 280 chars | the method, above |
| nodes per map | 500 | a brainstorm, not a database. Past it: promote branches, or start a second map |
| depth | 8 | the ceiling the initiative folder tree uses — past it nobody can read the shape |
| nodes per call | 50 | one agent turn. The loop runs inside the write transaction that serializes every claim in the store |

The whole map comes back in one read (`GET /v1/mindmaps/{id}`) precisely because
of the 500 cap: a canvas cannot draw half a tree, and paging one would be a
worse contract than bounding it.

## Placement, order and shape

`position` orders siblings and is gapped (1000, 2000, …), so inserting between two
nodes is one write rather than a renumber of the ring.

`at` is hand placement — `{x, y}` or `null` to let the layout place it. Null by
default, which keeps a map growing at typing speed tidy; a map that *has* been
arranged stays exactly where it was left, and `at: null` hands one node back to
the layout ("tidy up" clears them all). The pair is one field rather than two
nullable numbers, because half a coordinate places nothing.

Hanging a node off its own descendant is refused (`mindmap.cycle`). It would cut
that branch off the map, and it is exactly what a drag makes easy to try.

## What reaches the event log

`mindmap_created`, `mindmap_grown` (one event per batch, not per node — ten nodes
from an agent turn are one act of brainstorming), `mindmap_moved` (a reparent
only), `mindmap_pruned`, `mindmap_promoted`, `mindmap_updated`, `mindmap_deleted`.

Text and placement edits reach nothing. They change constantly while somebody is
thinking, and an event per keystroke-batch would bury every other event in the
project under one person's typing.

## Surfaces

- **CLI** — `takomo mindmap new|ls|show|add|promote|rm`
- **MCP** — `takomo_mindmap_new`, `takomo_mindmap_grow`, `takomo_mindmap_show`,
  `takomo_mindmap_list`, `takomo_mindmap_promote`. The two reads are free.
- **REST** — `/v1/mindmaps`, see `spec/openapi.yaml`
- **A canvas** at `/mindmaps` — drag a node onto another to reparent it, into
  space to pin it; Enter for the next thought, Tab to go deeper, F2 to retype,
  Delete to prune. A node with no hand placement is laid out automatically, which
  keeps a map growing at typing speed tidy, and "tidy up" hands every pinned node
  back to the layout. On a phone the same tree is an indented list — a better
  shape for the screen than a pinch-zoom canvas, not a consolation prize.

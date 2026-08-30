// The live map: one Y.Doc, one socket, and everybody's edits arriving as they
// happen.
//
// This is `/documents`' Editor.tsx applied to a canvas, and the shape is
// deliberately the same one — a page mints the session, this component owns the
// document and the provider, and the surfaces below it are renderers that never
// learn where their nodes came from.
//
// The two rules worth carrying over verbatim:
//
//   * base, room and params are passed to the provider SEPARATELY. `y-websocket`
//     composes `serverUrl + "/" + room + "?" + params` itself, so a finished URL
//     puts the room after the query string and the socket silently goes to a path
//     the server does not route.
//   * connection state is seeded from what the provider already IS, not from the
//     next event. The provider connects inside the `useMemo`, so a `status` event
//     can land before this effect subscribes — which left the documents header
//     reading "Connecting…" over a visibly working document.
//
// And one rule of its own: there is no save button and no dirty state, because
// the honest question is not "did my change save" but "am I connected".
import { useCallback, useEffect, useMemo, useState } from 'react'
import { WebsocketProvider } from 'y-websocket'
import * as Y from 'yjs'

import { Canvas, type CanvasLabels, type CanvasMode, type CanvasPeer } from '@/components/mindmap/Canvas'
import { Details, type DetailsLabels } from '@/components/mindmap/Details'
import { Outline, type OutlineLabels } from '@/components/mindmap/Outline'
import { PruneDialog, type PruneDialogLabels } from '@/components/mindmap/PruneDialog'
import {
  MAX_NODES,
  MAX_RELATIONSHIPS,
  descendantCounts as allDescendantCounts,
  descendantsOf,
  visibleNodes,
  type MapNode,
  type NodeFields,
  type Relationship,
} from '@/lib/mindmap-doc'
import {
  addAttachment,
  createNode,
  createRelationship,
  deleteRelationship,
  deleteSubtree,
  nodesMap,
  place,
  readNodes,
  readRelationships,
  relationshipsMap,
  removeAttachment,
  reparent,
  setFields,
  setNotes,
  setTitle,
  tidyAll,
} from '@/lib/mindmap-crdt'
import type { Point } from '@/lib/mindmap-layout'
import { mindmapSyncBase, type MindmapSession } from '@/lib/mindmaps'

/** Caret colours. Fixed palette, picked by hashing the name so it is stable —
 *  the same function `/documents` uses, for the same reason. */
const CARET_COLORS = ['#2563eb', '#059669', '#d97706', '#dc2626', '#7c3aed', '#0891b2', '#c026d3']

function colorFor(name: string): string {
  let hash = 0
  for (let i = 0; i < name.length; i++) hash = (hash * 31 + name.charCodeAt(i)) | 0
  return CARET_COLORS[Math.abs(hash) % CARET_COLORS.length] ?? '#2563eb'
}

export type ConnectionState = 'connecting' | 'connected' | 'disconnected'

/** Fold, pan-shape and zoom are per-viewer. Collapsing a branch must not collapse
 *  it under somebody else mid-conversation, so none of this is in the document. */
const foldKey = (id: string) => `takomo.mindmap.fold.${id}`
const MODE_KEY = 'takomo.mindmap.mode'

function loadFold(id: string): Set<string> {
  try {
    const raw = localStorage.getItem(foldKey(id))
    const parsed: unknown = raw ? JSON.parse(raw) : []
    return new Set(Array.isArray(parsed) ? parsed.filter((x) => typeof x === 'string') : [])
  } catch {
    return new Set()
  }
}

export interface LiveLabels {
  branch: string
  readOnly: string
  newThought: string
  relationLabelPrompt: string
  capNodes: string
  capRelationships: string
  needWrite: string
}

export interface LiveProps {
  session: MindmapSession
  /** The map's title. The root box is the map, not a node. */
  title: string
  onConnection: (state: ConnectionState) => void
  onPeers: (names: string[]) => void
  /** So the page's promote buttons know what is selected. */
  onSelected: (node: MapNode | null) => void
  onError: (message: string) => void
  labels: LiveLabels
  canvasLabels: CanvasLabels
  outlineLabels: OutlineLabels
  detailsLabels: DetailsLabels
  pruneLabels: PruneDialogLabels
}

export default function Live({
  session,
  title,
  onConnection,
  onPeers,
  onSelected,
  onError,
  labels,
  canvasLabels,
  outlineLabels,
  detailsLabels,
  pruneLabels,
}: LiveProps) {
  // One Y.Doc and one provider per map, rebuilt only when the ticket changes.
  // Recreating either on an unrelated render would drop the connection and resync
  // from scratch under somebody's cursor.
  const { ydoc, provider } = useMemo(() => {
    const ydoc = new Y.Doc()
    const provider = new WebsocketProvider(mindmapSyncBase(session), session.room, ydoc, {
      params: { ticket: session.token },
      connect: true,
    })
    return { ydoc, provider }
  }, [session])

  const [nodes, setNodes] = useState<MapNode[]>([])
  const [relationships, setRelationships] = useState<Relationship[]>([])
  const [selected, setSelected] = useState<string | null>(null)
  const [peers, setPeers] = useState<CanvasPeer[]>([])
  const [collapsed, setCollapsed] = useState<Set<string>>(() => loadFold(session.mindmap))
  const [mode, setMode] = useState<CanvasMode>(() =>
    localStorage.getItem(MODE_KEY) === 'tidy' ? 'tidy' : 'radial',
  )
  const [relationFrom, setRelationFrom] = useState<string | null>(null)
  const [pruning, setPruning] = useState<string | null>(null)

  const canWrite = session.can_write

  // The document is the source; React state is a projection of it. `observeDeep`
  // rather than `observe` because a title is a Y.Text inside a Y.Map inside a
  // Y.Map, and a shallow observer would never see somebody typing.
  useEffect(() => {
    const read = () => {
      const next = readNodes(ydoc)
      setNodes(next)
      setRelationships(readRelationships(ydoc, next))
    }
    read()
    const nm = nodesMap(ydoc)
    const rm = relationshipsMap(ydoc)
    nm.observeDeep(read)
    rm.observeDeep(read)
    return () => {
      nm.unobserveDeep(read)
      rm.unobserveDeep(read)
    }
  }, [ydoc])

  useEffect(() => {
    const onStatus = ({ status }: { status: string }) => {
      onConnection(
        status === 'connected' ? 'connected' : status === 'connecting' ? 'connecting' : 'disconnected',
      )
    }
    const onSynced = (isSynced: boolean) => {
      if (isSynced) onConnection('connected')
    }
    // Seed from what the provider already is: it connected during the `useMemo`
    // above, so a `status` event can have landed before this subscription did.
    if (provider.wsconnected) onConnection('connected')

    const onAwareness = () => {
      const seen: CanvasPeer[] = []
      provider.awareness.getStates().forEach((state, clientId) => {
        if (clientId === provider.awareness.clientID) return
        const s = state as { user?: { name?: string }; mm?: { selected?: string | null } }
        if (!s.user?.name) return
        seen.push({
          name: s.user.name,
          color: colorFor(s.user.name),
          selected: s.mm?.selected ?? null,
        })
      })
      setPeers(seen)
      onPeers(seen.map((p) => p.name))
    }

    provider.on('status', onStatus)
    provider.on('sync', onSynced)
    provider.awareness.on('change', onAwareness)
    provider.awareness.setLocalStateField('user', {
      name: session.display,
      color: colorFor(session.display),
    })

    return () => {
      provider.off('status', onStatus)
      provider.off('sync', onSynced)
      provider.awareness.off('change', onAwareness)
      provider.destroy()
      ydoc.destroy()
    }
  }, [provider, ydoc, session.display, onConnection, onPeers])

  // Which node this viewer is on, so a collaborator's ring follows them.
  useEffect(() => {
    provider.awareness.setLocalStateField('mm', { selected })
  }, [provider, selected])

  useEffect(() => {
    localStorage.setItem(foldKey(session.mindmap), JSON.stringify([...collapsed]))
  }, [session.mindmap, collapsed])

  const selectedNode = useMemo(
    () => nodes.find((n) => n.id === selected) ?? null,
    [nodes, selected],
  )
  useEffect(() => onSelected(selectedNode), [selectedNode, onSelected])

  const shown = useMemo(() => visibleNodes(nodes, collapsed), [nodes, collapsed])
  // One post-order pass, not one full index rebuild per node — this recomputes
  // on every remote keystroke.
  const descendantCounts = useMemo(() => allDescendantCounts(nodes), [nodes])
  const titleOf = useMemo(() => new Map(nodes.map((n) => [n.id, n.title])), [nodes])

  const guard = useCallback((): boolean => {
    if (canWrite) return true
    onError(labels.needWrite)
    return false
  }, [canWrite, onError, labels.needWrite])

  const add = useCallback(
    (parent: string | null, after: string | null) => {
      if (!guard()) return
      const id = createNode(ydoc, { parent, after, title: labels.newThought, by: session.display })
      if (!id) {
        onError(labels.capNodes.replace('{max}', String(MAX_NODES)))
        return
      }
      setSelected(id)
    },
    [guard, ydoc, labels.newThought, labels.capNodes, session.display, onError],
  )

  const onSibling = useCallback(
    (id: string) => {
      const node = nodes.find((n) => n.id === id)
      if (node) add(node.parent, id)
    },
    [nodes, add],
  )
  const onChild = useCallback((id: string) => add(id, null), [add])

  const onTitle = useCallback(
    (id: string, next: string) => {
      if (!guard()) return
      const trimmed = next.trim()
      // An emptied node is a deletion in every outliner, and typing over a
      // first-draft thought then clearing it is the commonest way to say
      // "actually, no" — but it still goes through the same questions.
      if (!trimmed) {
        setPruning(id)
        return
      }
      setTitle(ydoc, id, trimmed)
    },
    [guard, ydoc],
  )

  const onReparent = useCallback(
    (id: string, parent: string) => {
      if (!guard()) return
      // The canvas already refuses a drop onto a descendant, but it can only see
      // the nodes it was GIVEN — and a folded branch is not among them. Checked
      // again here against the whole tree, so a fold cannot be used to tie a knot
      // the reader would then watch normalisation untie.
      if (descendantsOf(nodes, id).includes(parent)) return
      reparent(ydoc, id, parent)
    },
    [guard, ydoc, nodes],
  )
  const onPlace = useCallback(
    (id: string, at: Point) => {
      if (guard()) place(ydoc, id, at)
    },
    [guard, ydoc],
  )
  const onTidy = useCallback(() => {
    if (guard()) tidyAll(ydoc)
  }, [guard, ydoc])

  const onFields = useCallback(
    (id: string, fields: Partial<NodeFields>) => {
      if (guard()) setFields(ydoc, id, fields)
    },
    [guard, ydoc],
  )
  const onNotes = useCallback(
    (id: string, notes: string) => {
      if (guard()) setNotes(ydoc, id, notes)
    },
    [guard, ydoc],
  )

  const onToggleCollapse = useCallback((id: string) => {
    setCollapsed((current) => {
      const next = new Set(current)
      if (!next.delete(id)) next.add(id)
      return next
    })
  }, [])

  // Drawing a relation is two clicks and a label, and the label is asked for
  // rather than defaulted: an unlabelled cross-link is a line nobody can read
  // later.
  const onRelationTarget = useCallback(
    (target: string) => {
      const from = relationFrom
      setRelationFrom(null)
      if (!from || from === target || !guard()) return
      const label = window.prompt(labels.relationLabelPrompt, '')
      if (label === null) return
      if (!createRelationship(ydoc, from, target, label.trim())) {
        onError(labels.capRelationships.replace('{max}', String(MAX_RELATIONSHIPS)))
      }
    },
    [relationFrom, guard, ydoc, labels.relationLabelPrompt, labels.capRelationships, onError],
  )

  const pruneTarget = pruning ? (nodes.find((n) => n.id === pruning) ?? null) : null

  return (
    <>
      {/* A first branch has to come from somewhere: an empty map has no node to
          press Enter on, and the phone list has no keyboard shortcuts at all. */}
      <div className="border-b-border-soft flex shrink-0 items-center gap-2 border-b px-4 py-1.5">
        <button
          type="button"
          disabled={!canWrite}
          onClick={() => add(null, null)}
          className="border-border text-muted-foreground hover:text-foreground cursor-pointer rounded-md border px-2.5 py-1 text-[12px] font-[650] disabled:opacity-40"
        >
          + {labels.branch}
        </button>
        {!canWrite && (
          <span className="text-muted-foreground text-[11.5px]">{labels.readOnly}</span>
        )}
      </div>

      {/* The canvas is the desktop surface; a phone gets the same tree as a list,
          which is a better shape for the screen rather than a consolation prize. */}
      <div className="hidden min-h-0 flex-1 md:flex">
        <Canvas
          className="flex"
          title={title}
          nodes={shown}
          relationships={relationships}
          collapsed={collapsed}
          descendantCounts={descendantCounts}
          onToggleCollapse={onToggleCollapse}
          peers={peers}
          selected={selected}
          onSelect={setSelected}
          onTitle={onTitle}
          onSibling={onSibling}
          onChild={onChild}
          onDelete={setPruning}
          onReparent={onReparent}
          onPlace={onPlace}
          onTidy={onTidy}
          mode={mode}
          onMode={(m) => {
            setMode(m)
            localStorage.setItem(MODE_KEY, m)
          }}
          relationFrom={relationFrom}
          onRelationTarget={onRelationTarget}
          onCancelRelation={() => setRelationFrom(null)}
          canWrite={canWrite}
          labels={canvasLabels}
        />
        {selectedNode && (
          <aside className="border-l-border-soft w-full shrink-0 overflow-y-auto border-l md:w-80">
            <Details
              node={selectedNode}
              relations={relationships.filter(
                (r) => r.from === selectedNode.id || r.to === selectedNode.id,
              )}
              titleOf={titleOf}
              canWrite={canWrite}
              drawingRelation={relationFrom === selectedNode.id}
              onNotes={onNotes}
              onFields={onFields}
              onStartRelation={setRelationFrom}
              onCancelRelation={() => setRelationFrom(null)}
              onRemoveRelation={(id) => guard() && deleteRelationship(ydoc, id)}
              onAddAttachment={(id, draft) => {
                if (guard()) addAttachment(ydoc, id, draft)
              }}
              onRemoveAttachment={(id, att) => guard() && removeAttachment(ydoc, id, att)}
              onDelete={setPruning}
              labels={detailsLabels}
            />
          </aside>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto md:hidden">
        <Outline
          nodes={shown}
          selected={selected}
          onSelect={setSelected}
          onChild={onChild}
          onSibling={onSibling}
          labels={outlineLabels}
        />
      </div>

      <PruneDialog
        node={pruneTarget}
        descendants={pruneTarget ? descendantsOf(nodes, pruneTarget.id).length : 0}
        peers={peers.map((p) => p.name)}
        onOpenChange={(open) => !open && setPruning(null)}
        onConfirm={(id) => {
          setPruning(null)
          if (selected === id) setSelected(null)
          if (guard()) deleteSubtree(ydoc, id)
        }}
        labels={pruneLabels}
      />
    </>
  )
}

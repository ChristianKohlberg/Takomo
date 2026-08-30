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
//
// WHAT MOVED HERE, and why. There is no rail and no side panel any more: the
// canvas is the page. So this component is also where ⌘K lives, because the
// palette is scoped to the SELECTED NODE and the selection is document state, not
// page state. Which commands exist is decided by `lib/mindmap-commands.ts` — a
// pure function with tests, because jsdom could prove nothing about the overlay
// itself — and every command that does not apply is absent rather than disabled.
//
// The ⌘K listener is registered in the CAPTURE phase. React attaches its handlers
// at the root container, and the card's own fields stop propagation so a keystroke
// meant for a text box never reaches the canvas keyboard — which would also have
// swallowed ⌘K on its way to the window. Capturing runs first and cannot be
// stopped from inside.
import { useCallback, useEffect, useMemo, useState } from 'react'
import { WebsocketProvider } from 'y-websocket'
import * as Y from 'yjs'

import { Canvas, type CanvasLabels, type CanvasMode, type CanvasPeer } from '@/components/mindmap/Canvas'
import {
  CommandPalette,
  type CommandPaletteLabels,
  type PaletteItem,
} from '@/components/mindmap/CommandPalette'
import type { NodeCardLabels, Reveal } from '@/components/mindmap/NodeCard'
import {
  AttachmentsDialog,
  type AttachmentsDialogLabels,
} from '@/components/mindmap/AttachmentsDialog'
import type { MenuItem } from '@/components/mindmap/NodeMenu'
import type { PillVerb } from '@/components/mindmap/NodePill'
import { Outline, type OutlineLabels } from '@/components/mindmap/Outline'
import { PruneDialog, type PruneDialogLabels } from '@/components/mindmap/PruneDialog'
import { DetachDialog, type DetachDialogLabels } from '@/components/mindmap/DetachDialog'
import {
  MAX_ATTACHMENTS,
  MAX_NODES,
  MAX_RELATIONSHIPS,
  ancestorsOf,
  descendantCounts as allDescendantCounts,
  descendantsOf,
  visibleNodes,
  type MapNode,
  type NodeFields,
  type Relationship,
} from '@/lib/mindmap-doc'
import {
  commandsFor,
  fuzzyRank,
  isTextEntry,
  menuVerbsFor,
  pillVerbsFor,
  type CommandId,
} from '@/lib/mindmap-commands'
import { cutTarget, foldSummary } from '@/lib/mindmap-lens'
import { draftsForDrop, type DropPayload } from '@/lib/mindmap-attach'
import {
  addAttachment,
  answerQuestion,
  createNode,
  createQuestion,
  detach,
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
  updateAttachment,
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

/**
 * One character per pill verb. The accessible name is always the command's own
 * label — a glyph is a reminder for somebody who already knows, never the name.
 */
const VERB_GLYPH: Partial<Record<CommandId, string>> = {
  'node.rename': '✎',
  'node.notes': '≋',
  'node.collapse': '⊟',
  'node.expand': '⊞',
  'node.relate': '⇢',
  'node.ask': '?',
}

export type ConnectionState = 'connecting' | 'connected' | 'disconnected'

/** Fold, pan-shape and zoom are per-viewer. Collapsing a branch must not collapse
 *  it under somebody else mid-conversation, so none of this is in the document. */
const foldKey = (id: string) => `takomo.mindmap.fold.${id}`
const MODE_KEY = 'takomo.mindmap.mode'
/** The trust lens is a lens, so it is off by default and remembered per viewer. */
const TRUST_KEY = 'takomo.mindmap.trust'

/** How many rows a second stage offers. A palette is a shortcut, not a browser. */
const MAX_ROWS = 12

function loadFold(id: string): Set<string> {
  try {
    const raw = localStorage.getItem(foldKey(id))
    const parsed: unknown = raw ? JSON.parse(raw) : []
    return new Set(Array.isArray(parsed) ? parsed.filter((x) => typeof x === 'string') : [])
  } catch {
    return new Set()
  }
}

/** What each command is called in the list. Keyed by the id the pure module owns. */
export type CommandLabels = Record<CommandId, string>

export interface LiveLabels {
  branch: string
  readOnly: string
  newThought: string
  relationLabelPrompt: string
  capNodes: string
  capRelationships: string
  needWrite: string
  /** Second stages: what the input is asking for. */
  gotoPlaceholder: string
  projectPlaceholder: string
  /** The gist written onto a dropped FILE, which is a pointer to something this
   *  document deliberately does not hold. */
  droppedFileGist: string
  /** Said when a drop does not fit under the per-node cap. `{max}`. */
  attachmentsFull: string
  /** The title a new question starts life with, before it is typed over. */
  newQuestion: string
  /** The trust lens toggle in the top strip, where a phone can reach it. */
  trustLens: string
}

export interface LiveProps {
  session: MindmapSession
  /** The map's title. The root box is the map, not a node. */
  title: string
  onConnection: (state: ConnectionState) => void
  onPeers: (names: string[]) => void
  onError: (message: string) => void
  /** Projects this token can reach — ⌘K is the only way to switch, now. */
  projects: { id: string; name: string }[]
  currentProject: string
  onProject: (id: string) => void
  /** Map-level commands. They go over REST, so they are the page's to run. */
  canManageMap: boolean
  onRenameMap: () => void
  onDeleteMap: () => void
  /** Turn the map into a tree of documents. */
  onWriteUp: () => void
  onPromote: (node: string, target: 'epic' | 'initiative') => void
  labels: LiveLabels
  canvasLabels: CanvasLabels
  outlineLabels: OutlineLabels
  cardLabels: NodeCardLabels
  pruneLabels: PruneDialogLabels
  detachLabels: DetachDialogLabels
  attachmentLabels: AttachmentsDialogLabels
  paletteLabels: CommandPaletteLabels
  commandLabels: CommandLabels
  /** One line under a command, where the command has consequences. */
  commandHints: Partial<Record<CommandId, string>>
}

type Stage = 'commands' | 'goto' | 'project'

export default function Live({
  session,
  title,
  onConnection,
  onPeers,
  onError,
  projects,
  currentProject,
  onProject,
  canManageMap,
  onRenameMap,
  onDeleteMap,
  onWriteUp,
  onPromote,
  labels,
  canvasLabels,
  outlineLabels,
  cardLabels,
  pruneLabels,
  detachLabels,
  attachmentLabels,
  paletteLabels,
  commandLabels,
  commandHints,
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
  /** The child whose line to its parent is being cut, or null. */
  const [cutting, setCutting] = useState<string | null>(null)
  const [trustLens, setTrustLens] = useState(() => localStorage.getItem(TRUST_KEY) === 'on')
  /** The node whose attachments are open in the manager, or null. */
  const [attaching, setAttaching] = useState<string | null>(null)

  // ⌘K, and the asks it hands the canvas. Each ask is cleared the moment the
  // canvas honours it, so none of them can fire twice.
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [stage, setStage] = useState<Stage>('commands')
  const [query, setQuery] = useState('')
  const [active, setActive] = useState<string | null>(null)
  const [centreNode, setCentreNode] = useState<string | null>(null)
  const [editNode, setEditNode] = useState<string | null>(null)
  const [fitRequest, setFitRequest] = useState<number | null>(null)
  const [reveal, setReveal] = useState<Reveal>(null)

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

  const shown = useMemo(() => visibleNodes(nodes, collapsed), [nodes, collapsed])
  // One post-order pass, not one full index rebuild per node — this recomputes
  // on every remote keystroke.
  const descendantCounts = useMemo(() => allDescendantCounts(nodes), [nodes])
  const titleOf = useMemo(() => new Map(nodes.map((n) => [n.id, n.title])), [nodes])
  const relationsFor = useCallback(
    (id: string) => relationships.filter((r) => r.from === id || r.to === id),
    [relationships],
  )

  /**
   * What each folded branch is holding.
   *
   * Computed here rather than in the canvas because the canvas is handed the
   * VISIBLE nodes, and the whole point of a fold summary is the nodes that are
   * not among them. Only folded branches are summarised, so the cost is bounded
   * by what this viewer folded rather than by the size of the map.
   */
  const foldSummaries = useMemo(() => {
    const out = new Map<string, ReturnType<typeof foldSummary>>()
    for (const id of collapsed) out.set(id, foldSummary(nodes, id))
    return out
  }, [nodes, collapsed])
  const foldSummaryOf = useCallback(
    (id: string) => foldSummaries.get(id) ?? null,
    [foldSummaries],
  )

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
  const onRemoveRelation = useCallback(
    (id: string) => {
      if (guard()) deleteRelationship(ydoc, id)
    },
    [guard, ydoc],
  )
  const onAddAttachment = useCallback(
    (id: string, draft: Parameters<typeof addAttachment>[2]) => {
      if (guard()) addAttachment(ydoc, id, draft)
    },
    [guard, ydoc],
  )
  const onUpdateAttachment = useCallback(
    (id: string, attachment: string, draft: Parameters<typeof addAttachment>[2]) => {
      if (guard()) updateAttachment(ydoc, id, attachment, draft)
    },
    [guard, ydoc],
  )
  const onRemoveAttachment = useCallback(
    (id: string, attachment: string) => {
      if (guard()) removeAttachment(ydoc, id, attachment)
    },
    [guard, ydoc],
  )

  /**
   * Something was dragged onto a node.
   *
   * The whole gesture is one pointer to something that lives elsewhere: a file
   * contributes its NAME and a guessed kind and nothing else, because bytes in a
   * shared document are bytes every peer replays on join. What does not fit
   * under the cap is reported rather than dropped quietly — a gesture that
   * silently keeps four of six files is worse than one that says so.
   */
  const onAttachDrop = useCallback(
    (id: string, payload: DropPayload) => {
      if (!guard()) return
      const node = nodes.find((n) => n.id === id)
      if (!node) return
      const { add, refused } = draftsForDrop(payload, node.attachments.length, {
        file: labels.droppedFileGist,
      })
      for (const draft of add) addAttachment(ydoc, id, draft)
      if (refused > 0) onError(labels.attachmentsFull.replace('{max}', String(MAX_ATTACHMENTS)))
      if (add.length > 0) setSelected(id)
    },
    [guard, nodes, ydoc, labels.droppedFileGist, labels.attachmentsFull, onError],
  )

  /**
   * A thought dropped into empty space.
   *
   * Created, pinned where it landed, selected and opened straight into its title
   * — four things, because the gesture means one thing: "this occurred to me,
   * here". It joins the first ring with no parent, which is the honest answer
   * while nobody knows where it goes yet.
   */
  const onCreateAt = useCallback(
    (at: Point) => {
      if (!guard()) return
      const id = createNode(ydoc, { parent: null, after: null, title: '', by: session.display })
      if (!id) {
        onError(labels.capNodes.replace('{max}', String(MAX_NODES)))
        return
      }
      place(ydoc, id, at)
      setSelected(id)
      setEditNode(id)
    },
    [guard, ydoc, session.display, onError, labels.capNodes],
  )

  /** Pose a question about the selected thought, and open its title to type it. */
  const onAsk = useCallback(
    (about: string | null) => {
      if (!guard()) return
      const id = createQuestion(ydoc, about, labels.newQuestion, session.display)
      if (!id) {
        onError(labels.capNodes.replace('{max}', String(MAX_NODES)))
        return
      }
      setSelected(id)
      setEditNode(id)
    },
    [guard, ydoc, session.display, onError, labels.capNodes, labels.newQuestion],
  )

  /**
   * Answer a question in a person's own words.
   *
   * The answer lands on the thought the question was about, that thought is
   * marked as looked at, and the question goes — so selection has to move with
   * it, or the page would sit on a node that no longer exists.
   */
  const onAnswerQuestion = useCallback(
    (id: string, answer: string) => {
      if (!guard()) return
      const landed = answerQuestion(ydoc, id, answer)
      if (landed) setSelected(landed)
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

  // ---- ⌘K -----------------------------------------------------------------

  const openPalette = useCallback(() => {
    setStage('commands')
    setQuery('')
    setActive(null)
    setPaletteOpen(true)
  }, [])
  const closePalette = useCallback(() => setPaletteOpen(false), [])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() !== 'k' || !(e.metaKey || e.ctrlKey)) return
      // Opening over somebody's typing would steal the keystroke; closing from
      // there is exactly what they want, so the toggle is allowed either way and
      // only OPENING is refused.
      if (!paletteOpen && isTextEntry(document.activeElement)) return
      e.preventDefault()
      if (paletteOpen) closePalette()
      else openPalette()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [paletteOpen, openPalette, closePalette])

  // One context, three consumers: ⌘K's list, the pill's verbs, and the
  // right-click menu. They differ in WHICH verbs they offer and never in what
  // applies, which is the only way three affordances over one node can stay
  // consistent with each other.
  const commandContext = useMemo(
    () => ({
      canWrite,
      canManageMap,
      nodeCount: nodes.length,
      projectCount: projects.length,
      node: selectedNode
        ? {
            id: selectedNode.id,
            title: selectedNode.title,
            promoted: !!selectedNode.promoted,
            attachments: selectedNode.attachments.length,
            hasChildren: (descendantCounts.get(selectedNode.id) ?? 0) > 0,
            collapsed: collapsed.has(selectedNode.id),
          }
        : null,
    }),
    [canWrite, canManageMap, nodes.length, projects.length, selectedNode, descendantCounts, collapsed],
  )

  const commands = useMemo(() => commandsFor(commandContext), [commandContext])

  const pillVerbs: PillVerb[] = useMemo(
    () =>
      pillVerbsFor(commandContext).map((id) => ({
        id,
        glyph: VERB_GLYPH[id] ?? '·',
        label: commandLabels[id],
      })),
    [commandContext, commandLabels],
  )

  const menuItems: MenuItem[] = useMemo(
    () =>
      menuVerbsFor(commandContext).map((id) => ({
        id,
        label: commandLabels[id],
        danger: id === 'node.delete',
      })),
    [commandContext, commandLabels],
  )

  const items: PaletteItem[] = useMemo(() => {
    if (stage === 'goto') {
      return fuzzyRank(nodes, (n) => n.title, query, MAX_ROWS).map((n) => ({
        id: n.id,
        label: n.title,
        hint: n.parent ? (titleOf.get(n.parent) ?? '') : '',
      }))
    }
    if (stage === 'project') {
      return fuzzyRank(projects, (p) => p.name || p.id, query, MAX_ROWS).map((p) => ({
        id: p.id,
        label: p.name || p.id,
        hint: p.id === currentProject ? paletteLabels.scopeMap : p.id,
      }))
    }
    return fuzzyRank(commands, (id) => commandLabels[id], query, commands.length).map((id) => ({
      id,
      label: commandLabels[id],
      hint: commandHints[id],
    }))
  }, [
    stage,
    query,
    nodes,
    titleOf,
    projects,
    currentProject,
    commands,
    commandLabels,
    commandHints,
    paletteLabels.scopeMap,
  ])

  /** Bring a node into view even when the viewer had folded the branch it is in. */
  const goTo = useCallback(
    (id: string) => {
      setCollapsed((current) => {
        const next = new Set(current)
        for (const parent of ancestorsOf(nodes, id)) next.delete(parent)
        return next
      })
      setSelected(id)
      setCentreNode(id)
    },
    [nodes],
  )

  const run = useCallback(
    (id: string) => {
      if (stage === 'goto') {
        closePalette()
        goTo(id)
        return
      }
      if (stage === 'project') {
        closePalette()
        onProject(id)
        return
      }
      const node = selected
      // A second stage keeps the palette open and re-aims the same input.
      if (id === 'map.goto' || id === 'map.project') {
        setStage(id === 'map.goto' ? 'goto' : 'project')
        setQuery('')
        setActive(null)
        return
      }
      closePalette()
      switch (id as CommandId) {
        case 'node.child':
          if (node) onChild(node)
          break
        case 'node.sibling':
          if (node) onSibling(node)
          break
        case 'node.rename':
          setEditNode(node)
          break
        case 'node.notes':
          setReveal('notes')
          break
        case 'node.attach':
          setAttaching(node)
          break
        case 'node.relate':
          setRelationFrom(node)
          break
        case 'node.ask':
          onAsk(node)
          break
        case 'node.promoteEpic':
          if (node) onPromote(node, 'epic')
          break
        case 'node.promoteInitiative':
          if (node) onPromote(node, 'initiative')
          break
        case 'node.collapse':
        case 'node.expand':
          if (node) onToggleCollapse(node)
          break
        case 'node.delete':
          setPruning(node)
          break
        case 'map.fit':
          setFitRequest(Date.now())
          break
        case 'map.trust':
          setTrustLens((on) => {
            localStorage.setItem(TRUST_KEY, on ? 'off' : 'on')
            return !on
          })
          break
        case 'map.tidy':
          onTidy()
          break
        case 'map.writeup':
          onWriteUp()
          break
        case 'map.rename':
          onRenameMap()
          break
        case 'map.delete':
          onDeleteMap()
          break
        default:
          break
      }
    },
    [
      stage,
      closePalette,
      goTo,
      onProject,
      selected,
      onChild,
      onSibling,
      onAsk,
      onPromote,
      onToggleCollapse,
      onTidy,
      onWriteUp,
      onRenameMap,
      onDeleteMap,
    ],
  )

  const onCentred = useCallback(() => setCentreNode(null), [])
  const onEditOpened = useCallback(() => setEditNode(null), [])
  const onFitted = useCallback(() => setFitRequest(null), [])
  const onRevealed = useCallback(() => setReveal(null), [])

  const pruneTarget = pruning ? (nodes.find((n) => n.id === pruning) ?? null) : null
  // Resolved from the live tree, so a line somebody else cut first closes the
  // dialog rather than confirming a cut that already happened.
  const cutT = cutting ? cutTarget(nodes, cutting) : null
  // Read from the live tree rather than captured when the badge was clicked, so
  // an attachment somebody else adds appears in the open dialog.
  const attachingNode = attaching ? (nodes.find((n) => n.id === attaching) ?? null) : null

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
        <button
          type="button"
          onClick={openPalette}
          className="border-border text-muted-foreground hover:text-foreground cursor-pointer rounded-md border px-2.5 py-1 font-mono text-[12px] font-[650]"
        >
          ⌘K
        </button>
        {/* The lens has a control on the canvas and a command on ⌘K, and a phone
            has neither a canvas nor a keyboard — so it gets the toggle here, and
            only here. */}
        <button
          type="button"
          aria-pressed={trustLens}
          onClick={() => {
            setTrustLens((on) => {
              localStorage.setItem(TRUST_KEY, on ? 'off' : 'on')
              return !on
            })
          }}
          className="border-border text-muted-foreground hover:text-foreground cursor-pointer rounded-md border px-2.5 py-1 text-[12px] font-[650] md:hidden"
        >
          ◍ {labels.trustLens}
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
          cardLabels={cardLabels}
          relationsFor={relationsFor}
          titleOf={titleOf}
          onNotes={onNotes}
          onFields={onFields}
          onRemoveRelation={onRemoveRelation}
          onOpenAttachments={setAttaching}
          onAttachDrop={onAttachDrop}
          pillVerbs={pillVerbs}
          menuItems={menuItems}
          onRunVerb={run}
          centreNode={centreNode}
          onCentred={onCentred}
          editNode={editNode}
          onEditOpened={onEditOpened}
          fitRequest={fitRequest}
          onFitted={onFitted}
          reveal={reveal}
          onRevealed={onRevealed}
          foldSummaryOf={foldSummaryOf}
          trustLens={trustLens}
          onTrustLens={(on) => {
            localStorage.setItem(TRUST_KEY, on ? 'on' : 'off')
            setTrustLens(on)
          }}
          onCreateAt={onCreateAt}
          onCutEdge={setCutting}
          onAnswerQuestion={onAnswerQuestion}
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto md:hidden">
        <Outline
          nodes={shown}
          selected={selected}
          canWrite={canWrite}
          onSelect={setSelected}
          onChild={onChild}
          onSibling={onSibling}
          onAttachments={setAttaching}
          onDelete={setPruning}
          onDetach={setCutting}
          foldSummaryOf={foldSummaryOf}
          trustLens={trustLens}
          labels={outlineLabels}
        />
      </div>

      {paletteOpen && (
        <CommandPalette
          scope={stage === 'commands' && selectedNode ? selectedNode.title : title}
          scopeKind={stage === 'commands' && selectedNode ? 'node' : 'map'}
          items={items}
          query={query}
          onQuery={setQuery}
          active={active}
          onActive={setActive}
          onRun={run}
          onClose={closePalette}
          labels={{
            ...paletteLabels,
            placeholder:
              stage === 'goto'
                ? labels.gotoPlaceholder
                : stage === 'project'
                  ? labels.projectPlaceholder
                  : paletteLabels.placeholder,
          }}
        />
      )}

      <AttachmentsDialog
        node={attachingNode}
        canWrite={canWrite}
        onOpenChange={(open) => !open && setAttaching(null)}
        onAdd={(id, draft) => onAddAttachment(id, draft)}
        onUpdate={onUpdateAttachment}
        onRemove={onRemoveAttachment}
        labels={attachmentLabels}
      />

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

      <DetachDialog
        edge={cutT}
        carries={cutT ? descendantsOf(nodes, cutT.child.id).length : 0}
        peers={peers.map((p) => p.name)}
        onOpenChange={(open) => !open && setCutting(null)}
        onConfirm={(id) => {
          setCutting(null)
          if (!guard()) return
          // Pinned where it is drawn rather than sent to the end of the first
          // ring: somebody clicked a LINE, and having the thought jump across
          // the map would lose the place they were reading.
          const node = nodes.find((n) => n.id === id)
          detach(ydoc, id, node?.at ?? null)
          setSelected(id)
        }}
        labels={detachLabels}
      />
    </>
  )
}

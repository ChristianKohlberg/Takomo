// The collaborative editor.
//
// **This module is loaded lazily and must stay that way.** Tiptap, ProseMirror,
// Yjs and the websocket provider together are larger than the entire rest of the
// app, and every other surface would pay for them on first paint. Splitting it
// is the whole reason `build.rs` embeds a generated asset manifest instead of
// four names — see `web/scripts/check-size.mjs`, which measures what index.html
// actually blocks on so a lazy chunk does not count against the budget.
//
// Nothing here writes prose to the server. The editor binds to a `Y.Doc` and the
// provider syncs it; persistence is the server's debounced flush. So there is no
// save button and no dirty state, which is the honest UI for a CRDT: the
// question "did my change save" is replaced by "am I connected", which is what
// the status line reports.
import { useCallback, useEffect, useMemo, useState } from 'react'
import { EditorContent, useEditor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import Collaboration from '@tiptap/extension-collaboration'
import CollaborationCaret from '@tiptap/extension-collaboration-caret'
import { WebsocketProvider } from 'y-websocket'
import * as Y from 'yjs'

import { BlockId } from '@/lib/block-id'
import { suggestionsFor, type SuggestionLabels } from '@/lib/doc-suggestions'
import { runAgent } from '@/lib/documents'
import { CommandMenu, type CommandMenuLabels } from './CommandMenu'
import { HighlightBlocks, setHighlightedBlocks } from '@/lib/block-highlight'
import { applyOps, blockText, parseProposal, touchedBlocks, type Proposal } from '@/lib/doc-ops'
import { syncBase, type DocSession } from '@/lib/documents'
import { Proposals, type ProposalsProps } from './Proposals'
import { Hint } from '@/components/Hint'

/** Caret colours. Fixed palette, picked by hashing the name so it is stable. */
const CARET_COLORS = [
  '#2563eb',
  '#059669',
  '#d97706',
  '#dc2626',
  '#7c3aed',
  '#0891b2',
  '#c026d3',
]

function colorFor(name: string): string {
  let hash = 0
  for (let i = 0; i < name.length; i++) hash = (hash * 31 + name.charCodeAt(i)) | 0
  // `?? CARET_COLORS[0]` only to satisfy noUncheckedIndexedAccess — the modulo
  // cannot leave the array.
  return CARET_COLORS[Math.abs(hash) % CARET_COLORS.length] ?? '#2563eb'
}

export type ConnectionState = 'connecting' | 'connected' | 'disconnected'

export interface EditorProps {
  session: DocSession
  /** Reported upward so the page can show it beside the title. */
  onConnection: (state: ConnectionState) => void
  /** Names of the other people currently in the document. */
  onPeers: (names: string[]) => void
  /** How many proposals are waiting on a person — shown in the header. */
  onPending: (n: number) => void
  /** The viewer's bearer token — the prompt bar calls `/run` with it. */
  token: string
  /** Whether this server has a document agent configured at all. */
  agentEnabled: boolean
  onError: (e: unknown) => void
  labels: {
    placeholder: string
    readOnly: string
    askHint: string
    agentOff: string
  }
  proposalLabels: ProposalsProps['labels']
  menuLabels: CommandMenuLabels
  suggestionLabels: SuggestionLabels
}

export default function Editor({
  session,
  onConnection,
  onPeers,
  onPending,
  token,
  agentEnabled,
  onError,
  labels,
  proposalLabels,
  menuLabels,
  suggestionLabels,
}: EditorProps) {
  // One Y.Doc and one provider per document, rebuilt only when the ticket
  // changes. Recreating either on an unrelated render would drop the connection
  // and, worse, resync from scratch mid-sentence.
  const { ydoc, provider } = useMemo(() => {
    const ydoc = new Y.Doc()
    // Base, room and params passed SEPARATELY. y-websocket composes
    // `serverUrl + "/" + room + "?" + params` itself, so handing it a finished
    // URL puts the room after the query string and the connection silently goes
    // to a path the server does not route.
    const provider = new WebsocketProvider(syncBase(session), session.room, ydoc, {
      params: { ticket: session.token },
      connect: true,
    })
    return { ydoc, provider }
  }, [session])

  const [ready, setReady] = useState(false)
  const [proposals, setProposals] = useState<Proposal[]>([])
  const [menuOpen, setMenuOpen] = useState(false)
  const [running, setRunning] = useState(false)
  const [context, setContext] = useState<{
    scopeId: string | null
    blockKind: string | null
    quote: string
  }>({ scopeId: null, blockKind: null, quote: '' })

  // Proposals live in the SAME Y.Doc as the prose, beside it rather than inside
  // it. That is what makes one appear in an open browser the moment an agent
  // writes it, and what keeps it there across a disconnect — a proposal parked
  // server-side until somebody reloaded would be a second source of truth about
  // the same document.
  const proposalMap = useMemo(() => ydoc.getMap<string>('proposals'), [ydoc])

  useEffect(() => {
    const read = () => {
      const out: Proposal[] = []
      proposalMap.forEach((raw) => {
        const p = parseProposal(raw)
        if (p) out.push(p)
      })
      setProposals(out)
      onPending(out.filter((p) => p.status === 'pending').length)
    }
    read()
    proposalMap.observe(read)
    return () => proposalMap.unobserve(read)
  }, [proposalMap, onPending])

  useEffect(() => {
    const onStatus = ({ status }: { status: string }) => {
      onConnection(
        status === 'connected'
          ? 'connected'
          : status === 'connecting'
            ? 'connecting'
            : 'disconnected',
      )
    }
    const onSynced = (isSynced: boolean) => {
      setReady(isSynced)
      // `sync` implies a live socket. Reported here as well as from `status`
      // because the two are not redundant: the provider connects during the
      // `useMemo` above, so a `status` event can land BEFORE this effect
      // subscribes — which left the header reading "Connecting…" over a
      // visibly working document.
      if (isSynced) onConnection('connected')
    }

    // …and for the same reason, seed from what the provider already is rather
    // than waiting for the next event.
    if (provider.wsconnected) onConnection('connected')
    const onAwareness = () => {
      const names: string[] = []
      provider.awareness.getStates().forEach((state, clientId) => {
        if (clientId === provider.awareness.clientID) return
        const user = (state as { user?: { name?: string } }).user
        if (user?.name) names.push(user.name)
      })
      onPeers(names)
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

  const editor = useEditor(
    {
      editable: session.can_write,
      extensions: [
        // Collaboration owns the document, so StarterKit's own history has to
        // go: an undo stack that does not know about remote edits would undo
        // somebody else's sentence. Yjs supplies a shared-aware UndoManager
        // through the Collaboration extension instead.
        StarterKit.configure({ undoRedo: false }),
        Collaboration.configure({ document: ydoc, field: 'prose' }),
        CollaborationCaret.configure({
          provider,
          user: { name: session.display, color: colorFor(session.display) },
        }),
        BlockId,
        HighlightBlocks,
      ],
      editorProps: {
        attributes: {
          class:
            'prose prose-neutral dark:prose-invert max-w-none focus:outline-none min-h-[60vh] px-1 py-2',
          'aria-label': labels.placeholder,
        },
      },
      // Tiptap warns without this when the editor mounts during SSR-shaped
      // renders; the app is client-only, so it is simply the correct answer.
      immediatelyRender: false,
    },
    [ydoc, provider, session.can_write],
  )

  // Keep the highlighted set in step with what is pending.
  useEffect(() => {
    if (!editor) return
    setHighlightedBlocks(editor.view, touchedBlocks(proposals))
  }, [editor, proposals])

  /** Record a decision on the proposal itself, so it stays readable afterwards. */
  const decide = useCallback(
    (p: Proposal, status: 'accepted' | 'rejected') => {
      const raw = proposalMap.get(p.id)
      const current = parseProposal(raw)
      if (!current || current.status !== 'pending') return false
      proposalMap.set(
        p.id,
        JSON.stringify({
          ...current,
          status,
          decided_by: session.display,
          decided_at: Date.now(),
        }),
      )
      return true
    },
    [proposalMap, session.display],
  )

  const onAccept = useCallback(
    (p: Proposal) => {
      if (!editor) return
      const tr = editor.state.tr
      const { applied } = applyOps(tr, editor.schema, p.ops)
      // The decision and the edit go in together. Marking it accepted without
      // applying it — or the reverse — leaves the document and its record
      // disagreeing, and the record is the only durable half.
      if (!decide(p, 'accepted')) return
      if (applied) editor.view.dispatch(tr)
    },
    [editor, decide],
  )

  const onReject = useCallback((p: Proposal) => decide(p, 'rejected') && undefined, [decide])

  const textFor = useCallback(
    (id: string) => (editor ? blockText(editor.state.doc, id) : null),
    [editor],
  )

  // ---- the prompt bar -----------------------------------------------------

  // The block the caret is in, and the words selected inside it.
  //
  // Held in React state and fed by the editor's own selection event, rather than
  // memoized on `editor.state.selection`: ProseMirror replaces the whole state
  // object on every transaction, so a dependency on it is not something React can
  // usefully compare — eslint is right to refuse it, and the memo would have gone
  // stale exactly when the caret moved.
  useEffect(() => {
    if (!editor) return
    const read = () => {
      const { state } = editor
      const { from, to, empty } = state.selection
      const $from = state.doc.resolve(from)
      // Depth 1 is the top-level block: the unit an op addresses.
      const node = $from.depth >= 1 ? $from.node(1) : null
      setContext({
        scopeId: (node?.attrs.id as string | undefined) ?? null,
        blockKind: node?.type.name ?? null,
        quote: empty ? '' : state.doc.textBetween(from, to, ' '),
      })
    }
    read()
    editor.on('selectionUpdate', read)
    editor.on('update', read)
    return () => {
      editor.off('selectionUpdate', read)
      editor.off('update', read)
    }
  }, [editor])

  const suggestions = useMemo(
    () =>
      suggestionsFor(
        { blockKind: context.blockKind, hasSelection: context.quote.length > 0 },
        suggestionLabels,
      ),
    [context.blockKind, context.quote, suggestionLabels],
  )

  const onRun = useCallback(
    async (instruction: string) => {
      if (running) return
      setRunning(true)
      try {
        // The quote goes into the instruction rather than being sent as a field:
        // the model is being told what was pointed at, and `scope` — which IS
        // enforced — carries the part that must not be trusted to prose.
        const asked = context.quote
          ? `${instruction}\n\nDie markierte Stelle lautet: „${context.quote}“`
          : instruction
        await runAgent(token, session.document, asked, context.scopeId ? [context.scopeId] : undefined)
        setMenuOpen(false)
      } catch (e) {
        onError(e)
      } finally {
        setRunning(false)
      }
    },
    [running, context.quote, context.scopeId, token, session.document, onError],
  )

  // ⌘K / Ctrl-K anywhere on the page. Registered on the document rather than the
  // editor so it works with the caret outside the prose too — which is exactly
  // when somebody wants a whole-document action.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        setMenuOpen((v) => !v)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  if (!editor) return null

  return (
    <div className="flex min-w-0 grow flex-col gap-4 overflow-y-auto md:flex-row">
      <div className="min-w-0 grow">
        {!session.can_write && (
          <p className="mb-3 rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-sm text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-200">
            {labels.readOnly}
          </p>
        )}
        <EditorContent editor={editor} className={ready ? '' : 'opacity-60'} />

        <Hint text={agentEnabled ? undefined : labels.agentOff}>
          <button
            type="button"
            disabled={!agentEnabled || !session.can_write}
            onClick={() => setMenuOpen(true)}
            className="border-border-soft text-muted-foreground hover:bg-accent/50 mt-4 flex w-full items-center gap-2 rounded-md border border-dashed px-3 py-2 text-left text-[13px] disabled:opacity-50"
          >
            <kbd className="border-border-soft rounded border px-1.5 py-0.5 text-[11px]">⌘K</kbd>
            {agentEnabled ? labels.askHint : labels.agentOff}
          </button>
        </Hint>
      </div>

      <CommandMenu
        open={menuOpen}
        onClose={() => setMenuOpen(false)}
        scopeId={context.scopeId}
        quote={context.quote}
        suggestions={suggestions}
        busy={running}
        onRun={onRun}
        labels={menuLabels}
      />

      {/* The review column. Below the document on a phone, beside it otherwise —
          one breakpoint, `md`, meaning "phone or not". */}
      <aside className="border-border-soft flex-none border-t pt-3 md:w-full md:max-w-80 md:border-t-0 md:border-l md:pt-0 md:pl-4">
        <Proposals
          proposals={proposals}
          textFor={textFor}
          canWrite={session.can_write}
          onAccept={onAccept}
          onReject={onReject}
          labels={proposalLabels}
        />
      </aside>
    </div>
  )
}

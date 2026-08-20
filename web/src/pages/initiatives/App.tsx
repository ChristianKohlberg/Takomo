// /initiatives — documents on the left, a readable document in the middle, and
// what you can do with a passage on the right.
//
// Initiatives are not work: no workflow, no claim, no lease, no ready queue.
// `status` is a label. Everything accumulates through append-only entries, each
// recording where it came from — and that is still true of everything this page
// does. Highlighting a sentence and commenting on it appends a `thread`;
// suggesting different words appends a proposed `view`; accepting one appends
// the amended prose plus a `decision`. Nothing here edits or deletes anything,
// which is what keeps the argument that produced the current text readable.
//
// The three columns exist because the two halves of the job are different. The
// explorer is navigation over many documents nested in folders; the middle is
// ONE continuous document, not three tabs, so a highlight is a single gesture
// that does not first require choosing a pane; the right is where a selection
// turns into an action.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { AppHeader } from '@/components/AppHeader'
import { AppShell } from '@/components/AppShell'
import { useNavCollapsed } from '@/hooks/useNavCollapsed'
import { useNavigate } from 'react-router'
import { isAuthError, loadProject, loadToken, saveProject, saveToken } from '@/lib/session'
import { EditableText } from '@/components/EditableText'
import { TokenGate } from '@/components/TokenGate'
import { useToast } from '@/components/Toaster'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Composer, type Draft, type PickedFile } from '@/components/initiatives/Composer'
import { CreateDialog } from '@/components/initiatives/CreateDialog'
import { DocumentBody } from '@/components/initiatives/DocumentBody'
import { EntryCard } from '@/components/initiatives/EntryCard'
import { Explorer, revealPath } from '@/components/initiatives/Explorer'
import { Minimap } from '@/components/initiatives/Minimap'
import { RenameDialog } from '@/components/initiatives/RenameDialog'
import { DeleteDialog } from '@/components/initiatives/DeleteDialog'
import { RollupStrip } from '@/components/initiatives/RollupStrip'
import { SelectionPane, type Operation } from '@/components/initiatives/SelectionPane'
import { SourceInspector } from '@/components/initiatives/SourceInspector'
import { SourcesFooter } from '@/components/initiatives/SourcesFooter'
import { OriginMasthead } from '@/components/initiatives/OriginMasthead'
import { VersionsStrip } from '@/components/initiatives/VersionsStrip'
import { resolveAnchor, type Anchor } from '@/lib/initiative-anchor'
import { fetchRoadmap, lane, laneVersions, laneWarnings, type Roadmap } from '@/lib/roadmap'
import { lanesOf } from '@/lib/initiative-map'
import {
  amendedView,
  buildDoc,
  citesOf,
  insertRunAt,
  paneText,
  serializeParagraphs,
  DECISION_KIND,
  PANES,
  THREAD_KIND,
  VIEW_KIND,
  type Amendment,
  type Pane,
  type Thread,
} from '@/lib/initiative-doc'
import { buildTree, folderPaths, normalizePath, pathOf, pruneTree } from '@/lib/initiative-tree'

import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { localInputToRfc3339 } from '@/lib/format'
import { listUsers } from '@/lib/users'
import { cn } from '@/lib/utils'
import {
  STATUSES,
  appendEntry,
  createInitiative,
  createQuestion,
  createTicket,
  countWaiting,
  downloadAttachment,
  isWaiting,
  listEntries,
  listInitiatives,
  listProjects,
  deleteInitiative,
  patchInitiative,
  readFileAsBase64,
  whoami,
  type CreateFields,
  type Entry,
  type Initiative,
  type InitiativeStatus,
  type Project,
} from '@/lib/initiatives'
import { STR } from './strings'

const LS_LANG = 'takomo.lang'
const LS_FOLDERS = 'takomo.initiatives.folders'

interface Me {
  actor: string
  scopes: string[]
}

const EMPTY_DRAFT: Draft = { kind: 'note', source: '', title: '', text: '', uri: '', origin: '' }

/** Entry kinds that ARE the document, and so are never offered as evidence. */
const DOCUMENT_KINDS = new Set([VIEW_KIND, THREAD_KIND, DECISION_KIND])

export function App() {
  const navigate = useNavigate()
  const { toast } = useToast()

  const [token, setToken] = useState(() => loadToken())
  const [lang, setLang] = useState<Locale>(() => detectLocale(localStorage.getItem(LS_LANG)))
  const [project, setProject] = useState(() => loadProject())
  const [gateError, setGateError] = useState('')

  const [me, setMe] = useState<Me>({ actor: '', scopes: [] })
  const [navCollapsed, setNavCollapsed] = useNavCollapsed()
  const [projects, setProjects] = useState<Project[]>([])
  const [items, setItems] = useState<Initiative[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [entries, setEntries] = useState<Entry[]>([])
  const [entryCursor, setEntryCursor] = useState<string | null>(null)
  const [roadmap, setRoadmap] = useState<Roadmap | undefined>(undefined)

  const [status, setStatus] = useState('')
  const [q, setQ] = useState('')
  const [waitingOnly, setWaitingOnly] = useState(false)
  const [creating, setCreating] = useState(false)
  const [busy, setBusy] = useState(false)
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT)
  const [file, setFile] = useState<PickedFile | null>(null)
  const [showLog, setShowLog] = useState(false)
  // The map is a view of the same project, not a separate page: it swaps the
  // document for the picture and leaves the explorer, so clicking a lane on the
  // map can put its document back on screen without a navigation.
  const [mapOpen, setMapOpen] = useState(false)
  /** The document whose title the rename modal is editing, or null. */
  const [renaming, setRenaming] = useState<Initiative | null>(null)
  /** The document the delete confirmation is about, or null. */
  const [deleting, setDeleting] = useState<Initiative | null>(null)

  // Which folders are open. Persisted because a tree that re-collapses on every
  // reload makes a deep document expensive to get back to.
  const [expanded, setExpanded] = useState<Set<string>>(() => {
    try {
      const raw = JSON.parse(localStorage.getItem(LS_FOLDERS) ?? '[]')
      return new Set(Array.isArray(raw) ? raw.filter((s): s is string => typeof s === 'string') : [])
    } catch {
      return new Set()
    }
  })

  // The live highlight, the note or suggestion opened from one, and the source
  // open in the inspector. All three are about the document being READ, so they
  // are cleared together when the selection changes.
  const [editingPane, setEditingPane] = useState<Pane | null>(null)
  const [anchor, setAnchor] = useState<Anchor | null>(null)
  const [focus, setFocus] = useState<{ pane: Pane; id: string } | null>(null)
  const [cite, setCite] = useState<{ entry: Entry; n: number } | null>(null)

  const t = useMemo(() => pick(STR, lang), [lang])
  const canWrite = me.scopes.includes('write')
  const actor = me.actor || 'human:web'

  // Reduced on every read, never stored — the same rule the server's `rollup`
  // follows, and for the same reason: a cached summary drifts from the entries
  // it summarises and there is no way to notice.
  const doc = useMemo(() => buildDoc(entries), [entries])

  const selected = useMemo(
    () => items.find((i) => i.id === selectedId) ?? null,
    [items, selectedId],
  )

  // Which project the roadmap is about. The project rather than the selected
  // document: the versions strip only ever needed the selection's project, but
  // the map is about the project as a whole and has to load with nothing
  // selected. When a project IS picked the list is already narrowed to it, so
  // the two agree.
  const roadmapProject = project || selected?.project

  // An initiative never closes, so "how far along is it" is not a question its
  // own status can answer. The answer is the epics filed under it, one per
  // version — joined here from the same roadmap response that carries them.
  const laneRollup = useMemo(() => lane(roadmap, selectedId ?? ''), [roadmap, selectedId])
  const laneVersionList = useMemo(
    () => laneVersions(roadmap, selectedId ?? ''),
    [roadmap, selectedId],
  )
  const laneFlags = useMemo(() => laneWarnings(laneRollup), [laneRollup])

  // The map's whole input, from the same roadmap response the versions strip
  // reads — one request answers both altitudes.
  const mapLanes = useMemo(() => lanesOf(roadmap), [roadmap])

  /** The map's root box: the project's name, falling back to its id. */
  const projectLabel = useMemo(() => {
    const p = roadmapProject
    if (!p) return ''
    return projects.find((x) => x.id === p)?.name || p
  }, [projects, roadmapProject])

  const fullTree = useMemo(() => buildTree(items), [items])

  // The tree is filtered client-side, unlike the status chips: folder structure
  // is derived here, so a server-side text filter would hand back a set of
  // documents with no way to know which folders they came from.
  const tree = useMemo(() => {
    const needle = q.trim().toLowerCase()
    if (!needle && !waitingOnly) return fullTree
    return pruneTree(fullTree, (i) => {
      if (waitingOnly && !isWaiting(i)) return false
      if (!needle) return true
      return `${i.title} ${i.summary ?? ''} ${pathOf(i)}`.toLowerCase().includes(needle)
    })
  }, [fullTree, q, waitingOnly])

  // Every folder that exists, not every folder currently VISIBLE: expand-all run
  // under a filter should leave the tree expanded when the filter comes off,
  // rather than half-open in a way nothing on screen explains.
  const allFolders = useMemo(() => folderPaths(fullTree), [fullTree])

  /**
   * How many documents want something from a person right now.
   *
   * Counted over every loaded document rather than over the filtered tree, so the
   * number does not shrink as you narrow the view — it is the answer to "is there
   * anything for me here", which a filter must not be able to change.
   */
  const waitingCount = useMemo(() => countWaiting(items), [items])

  /** Everything citable: entries that are evidence rather than document machinery. */
  const evidence = useMemo(() => entries.filter((e) => !DOCUMENT_KINDS.has(e.kind)), [entries])

  const openThreads = useMemo(
    () => PANES.flatMap((p) => doc.panes[p].threads.filter((th) => th.state !== 'resolved')),
    [doc],
  )
  const allPending = useMemo(() => PANES.flatMap((p) => doc.panes[p].pending), [doc])

  /** Which pane an entry id belongs to — the focus record needs it to resolve. */
  const paneOfEntry = useCallback(
    (id: string): Pane | null =>
      PANES.find(
        (p) =>
          doc.panes[p].threads.some((th) => th.entry.id === id) ||
          doc.panes[p].pending.some((a) => a.entry.id === id),
      ) ?? null,
    [doc],
  )

  // Derived from `focus` rather than stored, so accepting or rejecting makes the
  // pane fall closed by itself instead of leaving a decided proposal on screen.
  const focused = useMemo((): { thread: Thread } | { amendment: Amendment } | null => {
    if (!focus) return null
    const pd = doc.panes[focus.pane]
    const th = pd.threads.find((x) => x.entry.id === focus.id)
    if (th) return { thread: th }
    const am = pd.pending.find((x) => x.entry.id === focus.id)
    return am ? { amendment: am } : null
  }, [focus, doc])

  // A rejected token drops straight back to the gate; anything else is a toast
  // carrying the API's own message and remedy, verbatim.
  const handleErr = useCallback(
    (e: unknown) => {
      const err = e as { message?: string }
      if (isAuthError(e)) {
        setToken('')
        saveToken('')
        setGateError('')
        return
      }
      toast(err?.message || t.requestFailed, 'err')
    },
    [toast, t],
  )

  // ---- fetching -----------------------------------------------------------

  const fetchEntries = useCallback(
    async (id: string, replace: boolean, cursor?: string | null) => {
      const page = await listEntries(token, id, replace ? null : cursor)
      setEntries((prev) => (replace ? page.items : [...prev, ...page.items]))
      setEntryCursor(page.next_cursor ?? null)
      // The entry list carries the whole collection's rollup, so the selected
      // document's counts refresh from the same response.
      if (page.rollup) {
        setItems((prev) => prev.map((i) => (i.id === id ? { ...i, rollup: page.rollup } : i)))
      }
    },
    [token],
  )

  const fetchAll = useCallback(async () => {
    const page = await listInitiatives(token, { project, status })
    const next = page.items ?? []
    setItems(next)
    setSelectedId((cur) => {
      if (cur && !next.some((i) => i.id === cur)) {
        setEntries([])
        setEntryCursor(null)
        return null
      }
      return cur
    })
  }, [token, project, status])

  const clearReadingState = useCallback(() => {
    setEditingPane(null)
    setAnchor(null)
    setFocus(null)
    setCite(null)
  }, [])

  const select = useCallback(
    (id: string) => {
      setSelectedId(id)
      setEntries([])
      setEntryCursor(null)
      setShowLog(false)
      // The reading state belongs to the document you were reading, not to the
      // page: carrying an open citation across a selection would point the
      // inspector at a source the new document does not cite.
      clearReadingState()
      // Through the router and replacing, not pushing — one history entry per
      // document clicked is not what Back should mean.
      navigate({ hash: 'i=' + id }, { replace: true })
      fetchEntries(id, true).catch(handleErr)
    },
    [fetchEntries, handleErr, navigate, clearReadingState],
  )

  const toggleFolder = useCallback((path: string) => {
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      localStorage.setItem(LS_FOLDERS, JSON.stringify([...next]))
      return next
    })
  }, [])

  /**
   * Open or shut the whole tree.
   *
   * One control rather than two, because the two are never both useful: with
   * something open the offer is to shut it, and with nothing open there is
   * nothing to shut. It writes through the same key as `toggleFolder`, so the
   * state survives a reload exactly as a hand-opened folder does.
   */
  const setAllFolders = useCallback(
    (open: boolean) => {
      const next = open ? new Set(allFolders) : new Set<string>()
      localStorage.setItem(LS_FOLDERS, JSON.stringify([...next]))
      setExpanded(next)
    },
    [allFolders],
  )

  // Boot: projects + who am I, then the list.
  useEffect(() => {
    if (!token) return
    let cancelled = false
    ;(async () => {
      try {
        const [ps, who] = await Promise.all([
          listProjects(token).catch(() => [] as Project[]),
          whoami(token).catch(() => ({ actor: '', scopes: [] })),
        ])
        if (cancelled) return
        setProjects(ps)
        setMe({ actor: who.actor ?? '', scopes: who.scopes ?? [] })
        // The composer's source defaults to who you are: the common case is that
        // the person reading is the person the input came from.
        setDraft((d) => (d.source ? d : { ...d, source: who.actor ?? '' }))
      } catch (e) {
        handleErr(e)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [token, handleErr])

  useEffect(() => {
    if (!token) return
    fetchAll().catch(handleErr)
  }, [token, fetchAll, handleErr])

  // The lane's work, from the project roadmap. Kept deliberately soft: the strip
  // is supplementary to the document, so a roadmap that fails to load leaves it
  // unrendered instead of taking the page down with an error toast. A reader who
  // came here to read the argument still gets to.
  const refreshRoadmap = useCallback(
    async (project: string | undefined) => {
      if (!token || !project) {
        setRoadmap(undefined)
        return
      }
      try {
        setRoadmap(await fetchRoadmap(token, project))
      } catch {
        setRoadmap(undefined)
      }
    },
    [token],
  )

  useEffect(() => {
    void refreshRoadmap(roadmapProject)
  }, [refreshRoadmap, roadmapProject])

  // Deep link: `#i=<id>` selects a document, and reveals the folders above it —
  // a link into a collapsed branch that selects nothing visible is a broken link
  // as far as the reader is concerned.
  const selectRef = useRef(select)
  selectRef.current = select
  const selectedIdRef = useRef(selectedId)
  selectedIdRef.current = selectedId
  const itemsRef = useRef(items)
  itemsRef.current = items

  useEffect(() => {
    const apply = () => {
      const m = /(?:^#|&)i=([^&]+)/.exec(window.location.hash || '')
      const id = m?.[1] ? decodeURIComponent(m[1]) : null
      if (id && id !== selectedIdRef.current) {
        const hit = itemsRef.current.find((i) => i.id === id)
        if (hit) setExpanded((prev) => new Set([...prev, ...revealPath(hit)]))
        selectRef.current(id)
      }
    }
    apply()
    window.addEventListener('hashchange', apply)
    return () => window.removeEventListener('hashchange', apply)
  }, [items])

  // ---- writes -------------------------------------------------------------

  const guardWrite = useCallback(() => {
    if (canWrite) return true
    toast(t.writeNeeded, 'err')
    return false
  }, [canWrite, toast, t])

  const doPatch = useCallback(
    async (id: string, body: Parameters<typeof patchInitiative>[2], note: string) => {
      const updated = await patchInitiative(token, id, body)
      setItems((prev) => prev.map((i) => (i.id === id ? updated : i)))
      toast(note, 'success')
      return updated
    },
    [token, toast],
  )

  /** Move a document between folders — a metadata merge, and nothing else. */
  const doMove = useCallback(
    async (id: string, raw: string) => {
      const path = normalizePath(raw)
      try {
        await doPatch(id, { metadata_merge: { path: path || null } }, t.folderSaved)
        if (path) setExpanded((prev) => new Set([...prev, ...path.split('/').map((_, i, a) => a.slice(0, i + 1).join('/'))]))
      } catch (e) {
        handleErr(e)
      }
    },
    [doPatch, t, handleErr],
  )

  /** Rename a document from the tree. The same patch the inline title makes. */
  const doRename = useCallback(
    (id: string, title: string) => {
      doPatch(id, { title }, t.savedTitle).catch(handleErr)
    },
    [doPatch, t, handleErr],
  )

  /**
   * Delete a document, and tell the dialog whether the server refused.
   *
   * The refusal is returned rather than toasted: `conflict.initiative_has_checks`
   * is not a failure the reader should read and dismiss, it is a second question
   * — keep the checks, detached, or stop — and the dialog is where that gets
   * asked. Every other error is a real error and goes to the toast.
   */
  const doDelete = useCallback(
    async (id: string, force: boolean): Promise<string[] | null> => {
      try {
        const gone = await deleteInitiative(token, id, force)
        setItems((prev) => prev.filter((i) => i.id !== id))
        setDeleting(null)
        // Only the document being READ needs clearing; deleting another one from
        // the tree must not throw the reader out of what is open.
        if (selectedIdRef.current === id) {
          setSelectedId(null)
          setEntries([])
          setEntryCursor(null)
          clearReadingState()
          navigate({ hash: '' }, { replace: true })
        }
        toast(
          gone.detached_checks > 0
            ? t.delDoneDetached.replace('{n}', String(gone.detached_checks))
            : t.delDone,
          'success',
        )
        // The lane is gone from the roadmap, so the map has to be re-read.
        void refreshRoadmap(roadmapProject)
        return null
      } catch (e) {
        const err = e as { code?: string; details?: { checks?: unknown } }
        if (err.code === 'conflict.initiative_has_checks') {
          const checks = err.details?.checks
          return Array.isArray(checks) ? checks.map(String) : []
        }
        handleErr(e)
        return null
      }
    },
    [token, toast, t, handleErr, clearReadingState, navigate, refreshRoadmap, roadmapProject],
  )

  const doCreate = useCallback(
    async (fields: CreateFields) => {
      try {
        const ini = await createInitiative(token, fields)
        setCreating(false)
        toast(t.created, 'success')
        // Clear the filters before refetching: a new document is always `open`
        // and rarely matches whatever the reader had typed, and creating one
        // that then does not appear reads exactly like creating did nothing.
        if (status !== '' || q !== '') {
          setStatus('')
          setQ('')
        }
        const page = await listInitiatives(token, { project, status: '' })
        setItems(page.items ?? [])
        select(ini.id)
      } catch (e) {
        handleErr(e)
      }
    },
    [token, toast, t, project, status, q, select, handleErr],
  )

  // ---- document operations ------------------------------------------------
  //
  // Every one of them is an APPEND. The note, the ticket it became, the wording
  // it replaced and the person who decided are all still readable afterwards.

  const anchorMeta = (a: Anchor) => ({
    pane: a.pane,
    para: a.para,
    quote: a.quote,
    prefix: a.prefix,
    suffix: a.suffix,
  })

  /**
   * The people a question raised in this collection can be addressed to: the
   * members of its project. Empty on an instance with no directory, which hides
   * the picker and leaves "Ask a person" behaving as it did before.
   */
  const [people, setPeople] = useState<{ handle: string; label: string }[]>([])
  const peopleProject = selected?.project ?? ''
  useEffect(() => {
    if (!token || !peopleProject) {
      setPeople([])
      return
    }
    let cancelled = false
    listUsers(token, { project: peopleProject, limit: 200 })
      .then((page) => {
        if (cancelled) return
        setPeople(page.items.map((u) => ({ handle: u.handle, label: u.label })))
      })
      .catch(() => {
        if (!cancelled) setPeople([])
      })
    return () => {
      cancelled = true
    }
  }, [token, peopleProject])

  /** Run one of the five operations against the live highlight. */
  const runOp = useCallback(
    async (op: Operation, text: string, evidenceId?: string, assignee?: string) => {
      const ini = selected
      if (!selectedId || !ini || !anchor || !guardWrite()) return
      const pane = anchor.pane as Pane
      const paneDoc = doc.panes[pane]
      setBusy(true)
      try {
        if (op === 'comment') {
          await appendEntry(token, selectedId, {
            kind: THREAD_KIND,
            source: actor,
            text,
            meta: { ...anchorMeta(anchor), state: 'open' },
          })
          toast(t.commented, 'success')
        } else if (op === 'suggest') {
          await appendEntry(token, selectedId, {
            kind: VIEW_KIND,
            source: actor,
            text,
            // `cites: []` because a suggestion replaces WORDS, not the pane —
            // the marks around it are the live view's and stay its business.
            meta: { ...anchorMeta(anchor), cites: [], proposed: true },
          })
          toast(t.suggested, 'success')
        } else if (op === 'ticket' || op === 'ask') {
          const ticketId = await createTicket(token, {
            project: ini.project,
            title: text.slice(0, 120),
            body: `${text}\n\nRaised against “${anchor.quote}” in ${ini.title} (${selectedId}).\nAppend the answer to that initiative.`,
            tags: [`initiative:${selectedId}`],
          })
          if (op === 'ask') {
            // A question hangs off a ticket by design; `advisory` records the
            // decision without parking work nobody has claimed.
            await createQuestion(token, {
              ticket: ticketId,
              kind: 'clarify',
              mode: 'advisory',
              title: text.slice(0, 120),
              body: `About “${anchor.quote}” in ${ini.title}.`,
              // "Ask a person" becomes literal when one is named. Left unset it
              // goes to the open queue exactly as it always did.
              ...(assignee ? { assignee } : {}),
            })
          }
          await appendEntry(token, selectedId, {
            kind: THREAD_KIND,
            source: actor,
            text,
            meta: { ...anchorMeta(anchor), state: 'running', ticket: ticketId },
          })
          // The ticket carries this initiative's tag, so it just joined the lane —
          // refresh the versions strip rather than leaving it a request behind.
          void refreshRoadmap(ini.project)
          toast(op === 'ask' ? t.asked : `${t.dispatched} ${ticketId}`, 'success')
        } else if (op === 'cite') {
          const placed = resolveAnchor(paneText(paneDoc), anchor)
          const source = entries.find((e) => e.id === evidenceId)
          if (!placed || !source) {
            toast(t.orphanRefused, 'err')
            return
          }
          // Citing is a revision of the pane like any other: the mark goes in
          // after the words it supports and the whole pane is appended anew.
          const next = paneDoc.paragraphs.map((p, i) =>
            i === placed.para
              ? { ...p, runs: insertRunAt(p.runs, placed.end, { cite: 0, entry: source }) }
              : p,
          )
          const out = serializeParagraphs(next)
          await appendEntry(token, selectedId, {
            kind: VIEW_KIND,
            source: actor,
            text: out.text,
            meta: { pane, cites: out.cites },
          })
          toast(t.cited, 'success')
        }
        setAnchor(null)
        window.getSelection()?.removeAllRanges()
        await fetchEntries(selectedId, true)
      } catch (e) {
        handleErr(e)
      } finally {
        setBusy(false)
      }
    },
    [
      selected,
      selectedId,
      anchor,
      guardWrite,
      doc,
      token,
      actor,
      t,
      toast,
      entries,
      fetchEntries,
      refreshRoadmap,
      handleErr,
    ],
  )

  /**
   * Write or revise a pane.
   *
   * An append like every other document action: the new `view` supersedes the
   * old one for that pane and the earlier wording stays readable. The existing
   * `cites` are carried over unchanged, because the editor edits the pane's
   * SOURCE — where `[1]` indexes that array — and silently renumbering behind
   * the author would re-attribute their sentences to different sources.
   */
  const doSavePane = useCallback(
    async (pane: Pane, text: string) => {
      if (!selectedId || !guardWrite()) return
      const live = doc.panes[pane].entry
      setBusy(true)
      try {
        await appendEntry(token, selectedId, {
          kind: VIEW_KIND,
          source: actor,
          text,
          meta: { pane, cites: live ? citesOf(live) : [] },
        })
        setEditingPane(null)
        await fetchEntries(selectedId, true)
        toast(t.paneSaved, 'success')
      } catch (e) {
        handleErr(e)
      } finally {
        setBusy(false)
      }
    },
    [selectedId, guardWrite, doc, token, actor, t, fetchEntries, toast, handleErr],
  )

  /** File a ticket for an existing note, then supersede it as `running`. */
  const doDispatch = useCallback(
    async (thread: Thread) => {
      const ini = selected
      if (!selectedId || !ini || !guardWrite()) return
      setBusy(true)
      try {
        const ticketId = await createTicket(token, {
          project: ini.project,
          title: (thread.entry.text || t.dispatchFallbackTitle).slice(0, 120),
          body: `${thread.entry.text ?? ''}\n\nRaised in the margin of ${ini.title} (${selectedId}).\nAppend the answer to that initiative.`,
          tags: [`initiative:${selectedId}`],
        })
        await appendEntry(token, selectedId, {
          kind: THREAD_KIND,
          source: actor,
          text: thread.entry.text ?? '',
          meta: {
            ...(thread.anchor ? anchorMeta(thread.anchor) : { pane: '', para: thread.para }),
            state: 'running',
            ticket: ticketId,
            supersedes: thread.entry.id,
          },
        })
        await fetchEntries(selectedId, true)
        void refreshRoadmap(ini.project)
        toast(`${t.dispatched} ${ticketId}`, 'success')
      } catch (e) {
        handleErr(e)
      } finally {
        setBusy(false)
      }
    },
    [
      selected,
      selectedId,
      guardWrite,
      token,
      actor,
      t,
      fetchEntries,
      refreshRoadmap,
      toast,
      handleErr,
    ],
  )

  /** Settle a note without making work of it. */
  const doResolve = useCallback(
    async (thread: Thread) => {
      if (!selectedId || !guardWrite()) return
      setBusy(true)
      try {
        await appendEntry(token, selectedId, {
          kind: THREAD_KIND,
          source: actor,
          text: thread.entry.text ?? '',
          meta: {
            ...(thread.anchor ? anchorMeta(thread.anchor) : { pane: '', para: thread.para }),
            state: 'resolved',
            ...(thread.ticket ? { ticket: thread.ticket } : {}),
            supersedes: thread.entry.id,
          },
        })
        await fetchEntries(selectedId, true)
      } catch (e) {
        handleErr(e)
      } finally {
        setBusy(false)
      }
    },
    [selectedId, guardWrite, token, actor, fetchEntries, handleErr],
  )

  /**
   * Accept a suggestion: append the amended prose as a real `view`, then record
   * the decision. The view is what makes it live — the decision entry only says
   * who agreed and keeps the proposal from being offered twice.
   */
  const doAccept = useCallback(
    async (am: Amendment) => {
      const pane = paneOfEntry(am.entry.id)
      if (!selectedId || !pane || !guardWrite()) return
      const next = amendedView(doc.panes[pane], am)
      if (!next) {
        toast(t.orphanRefused, 'err')
        return
      }
      setBusy(true)
      try {
        await appendEntry(token, selectedId, {
          kind: VIEW_KIND,
          source: actor,
          text: next.text,
          meta: { pane, cites: next.cites, from: am.entry.id },
        })
        await appendEntry(token, selectedId, {
          kind: DECISION_KIND,
          source: actor,
          text: t.acceptedNote,
          meta: { accepts: am.entry.id, pane },
        })
        await fetchEntries(selectedId, true)
        toast(t.accepted, 'success')
      } catch (e) {
        handleErr(e)
      } finally {
        setBusy(false)
      }
    },
    [selectedId, paneOfEntry, guardWrite, doc, token, actor, t, fetchEntries, toast, handleErr],
  )

  /** Reject a suggestion: one `decision` entry. The live prose is untouched. */
  const doReject = useCallback(
    async (am: Amendment) => {
      const pane = paneOfEntry(am.entry.id)
      if (!selectedId || !pane || !guardWrite()) return
      setBusy(true)
      try {
        await appendEntry(token, selectedId, {
          kind: DECISION_KIND,
          source: actor,
          text: t.rejectedNote,
          meta: { rejects: am.entry.id, pane },
        })
        await fetchEntries(selectedId, true)
        toast(t.rejected, 'success')
      } catch (e) {
        handleErr(e)
      } finally {
        setBusy(false)
      }
    },
    [selectedId, paneOfEntry, guardWrite, token, actor, t, fetchEntries, toast, handleErr],
  )

  const doAppend = useCallback(async () => {
    if (!selectedId || !guardWrite()) return
    if (!draft.source.trim()) {
      toast(t.needSource, 'err')
      return
    }
    if (!draft.text.trim() && !file) {
      toast(t.needBody, 'err')
      return
    }
    const body: Parameters<typeof appendEntry>[2] = {
      kind: (draft.kind || 'note').trim(),
      source: draft.source.trim(),
    }
    if (draft.text.trim()) body.text = draft.text
    if (draft.title.trim()) body.title = draft.title.trim()
    if (draft.uri.trim()) body.source_uri = draft.uri.trim()
    const origin = localInputToRfc3339(draft.origin)
    if (origin) body.origin_at = origin
    if (file) {
      body.content_base64 = file.b64
      body.filename = file.name
      if (file.mime) body.mime = file.mime
    }

    setBusy(true)
    try {
      await appendEntry(token, selectedId, body)
      setDraft((d) => ({ ...EMPTY_DRAFT, kind: d.kind, source: d.source }))
      setFile(null)
      toast(t.appended, 'success')
      await fetchEntries(selectedId, true)
    } catch (e) {
      handleErr(e)
    } finally {
      setBusy(false)
    }
  }, [selectedId, guardWrite, draft, file, token, toast, t, fetchEntries, handleErr])

  const onPickFile = useCallback(
    (f: File | null) => {
      if (!f) {
        setFile(null)
        return
      }
      readFileAsBase64(f)
        .then(setFile)
        .catch(() => toast(t.fileUnreadable, 'err'))
    },
    [toast, t],
  )

  // ---- gate ---------------------------------------------------------------

  function signIn(tk: string) {
    saveToken(tk)
    setGateError('')
    setToken(tk)
  }

  function signOut() {
    saveToken('')
    setToken('')
    setItems([])
    setSelectedId(null)
    setEntries([])
  }

  if (!token) {
    return (
      <TokenGate
        title="takomo · initiatives"
        subtitle={t.gateTokenSub}
        tokenLabel={t.gateLabel}
        openLabel={t.gateOpen}
        emptyMessage={t.tokenNeeded}
        error={gateError}
        onSubmit={signIn}
      />
    )
  }

  const statusLabel = (s: InitiativeStatus) =>
    s === 'parked' ? t.statusParked : s === 'distilled' ? t.statusDistilled : t.statusOpen

  return (
    <AppShell
      rail={{
        onNavigate: navigate,
        current: 'initiatives',
        nav: {
          board: t.board,
          inbox: t.inbox,
          initiatives: t.initiatives,
          mindmaps: t.mindmaps,
          schedules: t.schedules,
          verification: t.verification,
          environments: t.environments,
        },
        // Free here — the page already knows, having counted the documents it is
        // showing. Carried on the current entry too so the number does not appear
        // to vanish when you follow it from the board.
        badges: { initiatives: waitingCount },
        projects: projects.map(({ id, name, archived, archived_at }) => ({
          id,
          name,
          archived,
          archived_at,
        })),
        project,
        onProject: (id) => {
          setProject(id)
          saveProject(id)
          setSelectedId(null)
          setEntries([])
          setEntryCursor(null)
          clearReadingState()
        },
        projectLabels: {
          project: t.project,
          search: t.projectSearch,
          noMatch: t.projectNoMatch,
          all: t.allProjects,
        },
        labels: {
          expand: t.navExpand,
          collapse: t.navCollapse,
          signOut: t.signOut,
          account: t.navAccount,
          settings: t.settings,
        },
        collapsed: navCollapsed,
        onCollapsed: setNavCollapsed,
        actor: me.actor,
        scopes: me.scopes,
        onSignOut: signOut,
      }}
    >
      <AppHeader
        title={t.initiatives}
        lang={lang}
        onLang={(l) => {
          setLang(l)
          localStorage.setItem(LS_LANG, l)
        }}
      >
        {/* A view of the same project rather than a page of its own, so it is a
            toggle here and not an entry in the nav rail: leaving it puts you
            back on the document you were reading. */}
        <Button
          variant={mapOpen ? 'default' : 'outline'}
          aria-pressed={mapOpen}
          onClick={() => setMapOpen((v) => !v)}
        >
          {mapOpen ? t.mapClose : t.mapOpen}
        </Button>
        {/* A document belongs to exactly one project, so this cannot act while
            the selection is "All projects". It shows its own state rather than
            answering a click with a toast. */}
        <Button
          aria-disabled={!project}
          title={project ? undefined : t.needProject}
          className={project ? undefined : 'opacity-55'}
          onClick={() => {
            if (!guardWrite()) return
            if (!project) {
              toast(t.needProject, 'err')
              return
            }
            setCreating(true)
          }}
        >
          + {t.newInitiative}
        </Button>
        <Button
          variant="outline"
          size="icon"
          title={t.refresh}
          onClick={() => fetchAll().catch(handleErr)}
        >
          ↻
        </Button>
      </AppHeader>

      <main
        className={cn(
          'grid min-h-0 flex-1 grid-cols-1 md:grid-rows-1',
          mapOpen
            ? 'grid-rows-[auto_1fr] md:grid-cols-[260px_1fr]'
            : 'grid-rows-[auto_1fr_auto] md:grid-cols-[260px_1fr_340px]',
        )}
      >
        {/* --- the explorer ------------------------------------------------ */}
        <aside className="bg-card border-b-border-soft md:border-r-border-soft flex max-h-[30vh] min-h-0 flex-col overflow-hidden border-b md:max-h-none md:border-r md:border-b-0">
          <div className="border-b-border-soft flex flex-col gap-1.5 border-b px-3 py-2">
            <Input
              value={q}
              onChange={(e) => setQ(e.target.value)}
              placeholder={t.explorerFilterPh}
              className="h-7 text-[12.5px]"
            />
            <div className="flex flex-wrap gap-1">
              <FilterChip label={t.all} on={status === ''} onClick={() => setStatus('')} />
              {STATUSES.map((s) => (
                <FilterChip
                  key={s}
                  label={statusLabel(s)}
                  on={status === s}
                  onClick={() => setStatus(s)}
                />
              ))}
              {/* A separate axis from status, so it toggles independently: status
                  is what the OWNER filed this as, waiting is what it wants from
                  the reader now. A parked document with an undecided amendment is
                  exactly the pair that makes them different questions.

                  The chip carries its own count rather than a sentence beside it:
                  the rail is 260px, where a sentence wraps, and "Waiting 3" says
                  the same thing without a plural to get wrong in two languages.
                  It appears only when something is actually waiting — but stays
                  while the filter is on, so turning it on can never make the
                  control that turned it on disappear. */}
              {(waitingCount > 0 || waitingOnly) && (
                <FilterChip
                  label={`${t.explorerWaitingOnly} ${waitingCount}`}
                  on={waitingOnly}
                  onClick={() => setWaitingOnly(!waitingOnly)}
                />
              )}
              {/* Only when there is a tree to open. A control that does nothing
                  visible is worse than an absent one on a 260px rail. */}
              {allFolders.length > 0 && (
                <button
                  type="button"
                  onClick={() => setAllFolders(expanded.size === 0)}
                  className="text-muted-foreground hover:text-foreground hover:border-ring border-border ml-auto cursor-pointer rounded-full border px-2 py-0.5 text-[11px] font-semibold"
                >
                  {expanded.size === 0 ? t.explorerExpandAll : t.explorerCollapseAll}
                </button>
              )}
            </div>
          </div>
          <div className="min-h-0 grow overflow-y-auto">
            <Explorer
              root={tree}
              selectedId={selectedId}
              expanded={expanded}
              onToggle={toggleFolder}
              onSelect={select}
              // Withheld from a reader who cannot write: the menu's only entry
              // would open a dialog whose save is refused.
              onRename={canWrite ? (i) => setRenaming(i) : undefined}
              onDelete={canWrite ? (i) => setDeleting(i) : undefined}
              labels={{
                empty: t.explorerEmpty,
                emptyHint: t.explorerEmptyHint,
                toggle: t.explorerToggle,
                unfiled: t.explorerUnfiled,
                menu: t.explorerMenu,
                rename: t.explorerRename,
                remove: t.explorerRemove,
                // Singular is its own string rather than a formatted "1", because
                // the two differ by more than a letter in German — "wartet" vs
                // "warten" — and a badge read aloud is the whole of what a screen
                // reader gets here.
                waitingNotes: (n) =>
                  n === 1 ? t.explorerWaitingNote : t.explorerWaitingNotes.replace('{n}', String(n)),
                waitingAmendments: (n) =>
                  n === 1
                    ? t.explorerWaitingAmendment
                    : t.explorerWaitingAmendments.replace('{n}', String(n)),
              }}
            />
          </div>
        </aside>

        {/* --- the map, or the document and what to do with it ------------- */}
        {mapOpen ? (
          <Minimap
            rootLabel={projectLabel}
            lanes={mapLanes}
            hasProject={Boolean(roadmapProject)}
            // Leaving the map on the document you asked for: the reason to click
            // a lane here is almost always to read it.
            onOpenInitiative={(id) => {
              setMapOpen(false)
              select(id)
            }}
            epicHref={(id) => `/board#t=${encodeURIComponent(id)}`}
            verificationHref="/verification"
            labels={{
              heading: t.mapHeading,
              needProject: t.mapNeedProject,
              needProjectHint: t.mapNeedProjectHint,
              empty: t.mapEmpty,
              emptyHint: t.mapEmptyHint,
              unfiled: t.mapUnfiled,
              unfiledHint: t.mapUnfiledHint,
              goTo: t.mapGoTo,
              openDocument: t.mapOpenDocument,
              openVerification: t.mapOpenVerification,
              versions: t.mapVersions,
              statistics: t.mapStatistics,
              close: t.mapClose2,
              done: t.mapDone,
              ready: t.mapReady,
              backlog: t.mapBacklog,
              awaiting: t.mapAwaiting,
              state: t.mapState,
              priority: t.mapPriority,
              openOnBoard: t.mapOpenOnBoard,
              noWork: t.mapNoWork,
              zoomIn: t.mapZoomIn,
              zoomOut: t.mapZoomOut,
              complete: t.mapComplete,
              pickEpic: t.mapPickEpic,
            }}
          />
        ) : (
          <>
            <section className="min-h-0 overflow-y-auto">
              {!selected ? (
                <Empty big={t.noneSelected} hint={t.noneSelectedHint} />
              ) : (
                <div className="max-w-215 px-6.5 pt-5 pb-10">
                  <div className="flex items-start gap-3">
                    <EditableText
                      key={selected.id + ':title'}
                      as="h1"
                      value={selected.title}
                      editable={canWrite}
                      required
                      className={cn(
                        'm-0 grow text-[22px] font-[740] tracking-[-0.02em] outline-none',
                        canWrite && 'focus:border-ring border-b border-dashed border-transparent',
                      )}
                      onCommit={(next) => doPatch(selected.id, { title: next }, t.savedTitle)}
                    />
                    <select
                      aria-label={t.aStatus}
                      value={selected.status}
                      disabled={!canWrite}
                      onChange={(e) =>
                        doPatch(
                          selected.id,
                          { status: e.target.value as InitiativeStatus },
                          t.savedStatus,
                        ).catch(handleErr)
                      }
                      className="bg-secondary text-secondary-foreground border-ring cursor-pointer appearance-none rounded-md border px-2.25 py-1 text-[11.5px] font-bold tracking-[0.04em] uppercase"
                    >
                      {STATUSES.map((s) => (
                        <option key={s} value={s}>
                          {statusLabel(s)}
                        </option>
                      ))}
                    </select>
                  </div>

                  <div className="text-muted-foreground flex flex-wrap items-center gap-x-2 font-mono text-[11.5px]">
                    <span>
                      {selected.id} · {selected.project}
                    </span>
                    <span aria-hidden="true">·</span>
                    {/* The folder is editable text like the title, because moving a
                        document is a metadata merge and nothing more. */}
                    <EditableText
                      key={selected.id + ':path'}
                      value={pathOf(selected)}
                      editable={canWrite}
                      placeholder={t.folderNone}
                      className={cn(
                        'outline-none',
                        canWrite && 'focus:border-ring border-b border-dashed border-transparent',
                      )}
                      onCommit={(next) => doMove(selected.id, next)}
                    />
                  </div>

                  <EditableText
                    key={selected.id + ':summary'}
                    value={selected.summary ?? ''}
                    editable={canWrite}
                    className={cn(
                      'text-foreground mt-2.25 mb-0 min-h-5 text-[14px] outline-none',
                      canWrite && 'focus:border-ring border-b border-dashed border-transparent',
                    )}
                    onCommit={(next) => doPatch(selected.id, { summary: next }, t.savedSummary)}
                  />

                  <RollupStrip
                    rollup={selected.rollup}
                    labels={{
                      entries: t.rEntries,
                      attachments: t.rAttachments,
                      chars: t.rChars,
                      size: t.rSize,
                      last: t.rLast,
                    }}
                  />

                  <VersionsStrip
                    lane={laneRollup}
                    versions={laneVersionList}
                    warnings={laneFlags}
                    epicHref={(id) => `/board#t=${encodeURIComponent(id)}`}
                    labels={{
                      heading: t.vHeading,
                      done: t.vDone,
                      ready: t.vReady,
                      backlog: t.vBacklog,
                      awaiting: t.vAwaiting,
                      empty: t.vEmpty,
                      emptyHint: t.vEmptyHint,
                      parkedWithReadyWork: t.vParkedWithReadyWork,
                    }}
                  />

                  <OriginMasthead
                    origins={doc.origins}
                    labels={{ heading: t.originHdr, wrote: t.wrote }}
                  />

                  {/* Rendered even when no pane exists yet: this is where a person
                      STARTS a document, and a surface that only appears once an
                      agent has written one is a surface a person can never begin. */}
                  {(doc.hasDocument || canWrite) && (
                    <>
                      <DocumentBody
                        doc={doc}
                        canWrite={canWrite}
                        editing={editingPane}
                        busy={busy}
                        onStartEdit={(pane) => {
                          setAnchor(null)
                          setFocus(null)
                          setEditingPane(pane)
                        }}
                        onCancelEdit={() => setEditingPane(null)}
                        onSavePane={(pane, text) => void doSavePane(pane, text)}
                        focusedSpan={focus?.id ?? null}
                        selectedSourceId={cite?.entry.id ?? null}
                        onSelect={(a) => {
                          setAnchor(a)
                          // A fresh highlight supersedes whatever note was open —
                          // two things claiming the right-hand pane at once is how
                          // a stale note ends up beside an unrelated selection.
                          if (a) setFocus(null)
                        }}
                        onOpenSpan={(pane, id) => {
                          setAnchor(null)
                          setFocus({ pane, id })
                        }}
                        onSelectSource={(entry, n) => setCite({ entry, n })}
                        labels={{
                          citation: t.aCitation,
                          uncited: t.uncited,
                          unwritten: t.paneUnwritten,
                          paneBusiness: t.paneBusiness,
                          paneTechnical: t.paneTechnical,
                          paneVerification: t.paneVerification,
                          orphanHeading: t.orphanHeading,
                          orphanHint: t.orphanHint,
                          suggestionMark: t.suggestionMark,
                          writePane: t.writePane,
                          revisePane: t.revisePane,
                          hint: t.paneEditorHint,
                          citesHeading: t.paneCites,
                          citesHint: t.paneCitesHint,
                          noCites: t.paneNoCites,
                          save: t.paneSave,
                          cancel: t.cancel,
                          working: t.working,
                        }}
                      />
                      {cite && (
                        <SourceInspector
                          entry={cite.entry}
                          n={cite.n}
                          onClose={() => setCite(null)}
                          onDownload={(e) => downloadAttachment(token, e).catch(handleErr)}
                          labels={{
                            hint: t.citeHint,
                            kind: t.eKind,
                            source: t.eSource,
                            wrote: t.wrote,
                            landed: t.landed,
                            download: t.download,
                            close: t.dismiss,
                          }}
                        />
                      )}
                      {doc.sources.length > 0 && (
                        <SourcesFooter
                          sources={doc.sources}
                          onSelect={(entry, n) => setCite({ entry, n })}
                          labels={{ heading: t.lineageHdr, wrote: t.wrote, landed: t.landed }}
                        />
                      )}
                    </>
                  )}

                  {/* The entry log stays reachable: the document is a reduction of
                      it, and a reduction you cannot check against its source is a
                      summary you have to take on faith. */}
                  <SectionHeader>
                    <button
                      type="button"
                      onClick={() => setShowLog((v) => !v)}
                      className="cursor-pointer"
                    >
                      {t.entriesHdr} {showLog ? '▾' : '▸'}
                    </button>
                  </SectionHeader>

                  {showLog && (
                    <>
                      {canWrite && (
                        <Composer
                          draft={draft}
                          onDraft={(patch) => setDraft((d) => ({ ...d, ...patch }))}
                          file={file}
                          onPickFile={onPickFile}
                          busy={busy}
                          onAppend={doAppend}
                          labels={{
                            kind: t.eKind,
                            kindHint: t.eKindHint,
                            source: t.eSource,
                            sourceHint: t.eSourceHint,
                            title: t.eTitle,
                            titlePh: t.eTitlePh,
                            uri: t.eSourceUri,
                            uriPh: t.eSourceUriPh,
                            text: t.eText,
                            textPh: t.eTextPh,
                            origin: t.eOrigin,
                            originHint: t.eOriginHint,
                            attach: t.attach,
                            attachClear: t.attachClear,
                            attachAria: t.aAttach,
                            append: t.append,
                            appending: t.appending,
                          }}
                        />
                      )}
                      {entries.length === 0 ? (
                        <Empty big={t.emptyEntries} hint={t.emptyEntriesHint} />
                      ) : (
                        <>
                          {entries.map((en) => (
                            <EntryCard
                              key={en.id}
                              entry={en}
                              labels={{ by: t.by, wrote: t.wrote, download: t.download }}
                              onDownload={(e) => downloadAttachment(token, e).catch(handleErr)}
                            />
                          ))}
                          {entryCursor && (
                            <Button
                              variant="outline"
                              onClick={() => fetchEntries(selected.id, false, entryCursor).catch(handleErr)}
                            >
                              {t.loadMore}
                            </Button>
                          )}
                        </>
                      )}
                    </>
                  )}
                </div>
              )}
            </section>

            {/* --- the operations pane ----------------------------------------- */}
            <aside className="bg-card border-t-border-soft md:border-l-border-soft min-h-0 overflow-y-auto border-t md:border-t-0 md:border-l">
              {selected && (
                <SelectionPane
                  canWrite={canWrite}
                  busy={busy}
                  anchor={anchor}
                  focused={focused}
                  openThreads={openThreads}
                  pending={allPending}
                  evidence={evidence}
                  onRun={runOp}
                  onOpenThread={(th) => {
                    const pane = paneOfEntry(th.entry.id)
                    if (pane) setFocus({ pane, id: th.entry.id })
                  }}
                  onOpenAmendment={(am) => {
                    const pane = paneOfEntry(am.entry.id)
                    if (pane) setFocus({ pane, id: am.entry.id })
                  }}
                  people={people}
                  onDispatch={doDispatch}
                  onResolve={doResolve}
                  onAccept={doAccept}
                  onReject={doReject}
                  onDismiss={() => {
                    setAnchor(null)
                    setFocus(null)
                    window.getSelection()?.removeAllRanges()
                  }}
                  labels={{
                    idleHeading: t.selIdle,
                    idleHint: t.selIdleHint,
                    selectionHeading: t.selHeading,
                    comment: t.opComment,
                    suggest: t.opSuggest,
                    ticket: t.opTicket,
                    ask: t.opAsk,
                    cite: t.opCite,
                    commentPh: t.opCommentPh,
                    suggestPh: t.opSuggestPh,
                    ticketPh: t.opTicketPh,
                    askPh: t.opAskPh,
                    askWho: t.opAskWho,
                    askAnyone: t.opAskAnyone,
                    citePh: t.opCitePh,
                    submit: t.opSubmit,
                    cancel: t.cancel,
                    working: t.working,
                    openNotes: t.openNotes,
                    pendingSuggestions: t.pendingSuggestions,
                    noteBy: t.noteBy,
                    dispatch: t.dispatch,
                    accept: t.accept,
                    reject: t.reject,
                    resolve: t.resolve,
                    threadOpen: t.threadOpen,
                    threadRunning: t.threadRunning,
                    threadResolved: t.threadResolved,
                    orphanWarning: t.orphanWarning,
                    ticketMade: t.ticketMade,
                    replaces: t.replaces,
                    with: t.replacesWith,
                    readOnly: t.writeNeeded,
                    noEvidence: t.noEvidence,
                  }}
                />
              )}
            </aside>
          </>
        )}
      </main>

      {/* Rename from the tree. Held here rather than in the Explorer so the
          write goes through the same patch the inline title uses, and a rename
          made from the tree updates the open document too. */}
      <RenameDialog
        open={renaming !== null}
        onOpenChange={(open) => {
          if (!open) setRenaming(null)
        }}
        value={renaming?.title ?? ''}
        subject={renaming?.id}
        onRename={(title) => {
          if (renaming) doRename(renaming.id, title)
          setRenaming(null)
        }}
        onInvalid={(m) => toast(m, 'err')}
        labels={{
          title: t.renameTitle,
          subtitle: t.renameSubtitle,
          field: t.renameField,
          placeholder: t.renamePh,
          save: t.renameSave,
          cancel: t.cancel,
          needTitle: t.needTitle,
        }}
      />

      <DeleteDialog
        initiative={deleting}
        onOpenChange={(open) => {
          if (!open) setDeleting(null)
        }}
        onDelete={doDelete}
        // Counted from the roadmap the page already holds, so the reader is told
        // how much work is filed under the lane BEFORE it disappears — not after,
        // in a toast.
        taggedTickets={deleting ? (lane(roadmap, deleting.id)?.total ?? 0) : 0}
        labels={{
          title: t.delTitle,
          body: t.delBody,
          contents: (n, a) =>
            t.delContents.replace('{n}', String(n)).replace('{a}', String(a)),
          stillWaiting: t.delWaiting,
          taggedWork: (n) => t.delTagged.replace('{n}', String(n)),
          irreversible: t.delIrreversible,
          checksTitle: t.delChecksTitle,
          checksBody: (n) => t.delChecksBody.replace('{n}', String(n)),
          checksForce: t.delChecksForce,
          confirm: t.delConfirm,
          confirmForce: t.delConfirmForce,
          cancel: t.cancel,
        }}
      />

      <CreateDialog
        open={creating}
        onOpenChange={setCreating}
        project={project}
        onCreate={doCreate}
        onInvalid={(m) => toast(m, 'err')}
        labels={{
          title: t.newInitiative,
          subtitle: t.newSub,
          fTitle: t.fTitle,
          fTitlePh: t.fTitlePh,
          fSummary: t.fSummary,
          fSummaryPh: t.fSummaryPh,
          fLabels: t.fLabels,
          fLabelsPh: t.fLabelsPh,
          fTags: t.fTags,
          fTagsPh: t.fTagsPh,
          fFolder: t.folder,
          fFolderPh: t.folderPh,
          create: t.create,
          cancel: t.cancel,
          needTitle: t.needTitle,
        }}
      />
    </AppShell>
  )
}

function FilterChip({ label, on, onClick }: { label: string; on: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      aria-pressed={on}
      onClick={onClick}
      className={cn(
        'cursor-pointer rounded-full px-2.25 py-0.5 text-[11.5px] font-semibold',
        on
          ? 'bg-primary text-primary-foreground'
          : 'bg-muted text-muted-foreground hover:text-foreground',
      )}
    >
      {label}
    </button>
  )
}

function SectionHeader({ children }: { children: React.ReactNode }) {
  return (
    <h2 className="text-muted-foreground mt-7 mb-2 text-[11.5px] font-bold tracking-[0.08em] uppercase">
      {children}
    </h2>
  )
}

function Empty({ big, hint }: { big: string; hint: string }) {
  return (
    <div className="px-6.5 py-10">
      <p className="text-foreground m-0 text-[15px] font-semibold">{big}</p>
      <p className="text-muted-foreground mt-1 mb-0 text-[13px]">{hint}</p>
    </div>
  )
}

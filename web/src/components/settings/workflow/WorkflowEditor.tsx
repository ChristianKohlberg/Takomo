// The workflow editor: canvas, inspector, preflight, apply.
//
// Everything is a DRAFT until Apply. Nothing here writes the project's workflow
// as a side effect of editing, because the thing being edited decides which
// ticket transitions are legal — an autosave would change what a running fleet
// is allowed to do, halfway through a thought.
//
// Validation is the server's. `validateWorkflow` runs the same check the PUT
// runs, debounced, so a draft cannot be called clean here and refused there.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import { autoLayout } from './layout'
import { Canvas, type CanvasSelection } from './Canvas'
import { Inspector, type InspectorLabels } from './Inspector'
import {
  getLayout,
  getStateCounts,
  putLayout,
  putProjectWorkflow,
  strandedStates,
  validateWorkflow,
  type Layout,
  type WorkflowDoc,
  type WorkflowEntry,
} from '@/lib/workflows'

export interface WorkflowEditorLabels extends InspectorLabels {
  title: string
  subtitle: string
  addState: string
  startFrom: string
  apply: string
  applying: string
  applied: string
  revert: string
  saveAs: string
  problems: string
  valid: string
  checking: string
  blockedTitle: string
  blockedBody: string
  blockedRow: string
  openBoard: string
  canvasInitial: string
  canvasClaimable: string
  canvasTerminal: string
  canvasHint: string
  readOnlyMsg: string
  newStateId: string
}

export interface WorkflowEditorProps {
  token: string
  project: string
  /** The project's current workflow, as loaded. */
  workflow: WorkflowDoc
  library: WorkflowEntry[]
  readOnly: boolean
  onApplied: (wf: WorkflowDoc) => void
  onError: (e: unknown) => void
  onSaveAs: (draft: WorkflowDoc, layout: Layout) => void
  labels: WorkflowEditorLabels
}

export function WorkflowEditor({
  token,
  project,
  workflow,
  library,
  readOnly,
  onApplied,
  onError,
  onSaveAs,
  labels,
}: WorkflowEditorProps) {
  const [draft, setDraft] = useState<WorkflowDoc>(workflow)
  // `layout` holds only the positions someone has actually chosen (stored, or
  // dragged this session). The layout the canvas RENDERS is derived below.
  const [layout, setLayout] = useState<Layout>({})
  const [selection, setSelection] = useState<CanvasSelection | null>(null)
  const [verdict, setVerdict] = useState<{ valid: boolean; problems: string[] } | null>(null)
  const [checking, setChecking] = useState(false)
  const [counts, setCounts] = useState<Record<string, number>>({})
  const [applying, setApplying] = useState(false)
  const [applied, setApplied] = useState(false)

  // The workflow prop changes when the caller switches project or re-reads after
  // an apply. Keyed on the project so a refetch of the SAME project does not
  // discard an in-progress draft.
  const loadedFor = useRef<string>('')
  useEffect(() => {
    if (loadedFor.current === project) return
    loadedFor.current = project
    setDraft(workflow)
    setSelection(null)
    setApplied(false)
    void getLayout(token, project)
      .then((r) => setLayout(r.layout ?? {}))
      .catch(() => setLayout({}))
    void getStateCounts(token, project).then(setCounts).catch(() => setCounts({}))
  }, [project, workflow, token])

  const dirty = useMemo(
    () => JSON.stringify(draft) !== JSON.stringify(workflow),
    [draft, workflow],
  )

  // The positions the canvas draws with, derived from the DRAFT rather than
  // stored alongside it.
  //
  // This is what makes a state without a position impossible. Storing the
  // rendered layout in state let the two fall out of step, and the canvas skips
  // any node it has no position for — so discarding a change after "Start
  // from…" left the draft on one workflow and the positions on another, and the
  // canvas rendered completely EMPTY. Deriving it means chosen positions are
  // honoured and everything else is placed, always.
  const view = useMemo(() => autoLayout(draft, layout), [draft, layout])

  // Ask the server whether the draft would be accepted. Debounced because it is
  // a request per keystroke otherwise, and the answer is only interesting once
  // typing pauses.
  useEffect(() => {
    if (!dirty) {
      setVerdict(null)
      return
    }
    let cancelled = false
    setChecking(true)
    const timer = setTimeout(() => {
      validateWorkflow(token, project, draft)
        .then((v) => !cancelled && setVerdict(v))
        .catch(() => !cancelled && setVerdict(null))
        .finally(() => !cancelled && setChecking(false))
    }, 400)
    return () => {
      cancelled = true
      clearTimeout(timer)
      setChecking(false)
    }
  }, [draft, dirty, project, token])

  const stranded = useMemo(() => strandedStates(draft, counts), [draft, counts])
  const canApply = dirty && !readOnly && stranded.length === 0 && verdict?.valid !== false

  const move = useCallback(
    (id: string, pos: { x: number; y: number }) => setLayout((l) => ({ ...l, [id]: pos })),
    [],
  )

  const connect = useCallback((from: string, to: string) => {
    setDraft((d) => {
      // The server rejects a duplicate (from, to) pair, and drawing the same
      // arrow twice is an easy slip on a dense graph.
      if (d.transitions.some((t) => t.from === from && t.to === to)) return d
      return { ...d, transitions: [...d.transitions, { from, to }] }
    })
  }, [])

  function addState() {
    // A generated id rather than a prompt: the inspector is where it gets its
    // real name, and a modal here would interrupt the drawing.
    let n = draft.states.length + 1
    while (draft.states.some((s) => s.id === `state_${n}`)) n++
    const id = `state_${n}`
    // No explicit position: the derived layout places it, which keeps a new
    // state in step with the graph instead of parked at a guessed coordinate.
    setDraft({ ...draft, states: [...draft.states, { id, category: 'todo' }] })
    setSelection({ kind: 'state', key: id })
  }

  function startFrom(entry: WorkflowEntry) {
    // The project keeps its own workflow NAME. A library entry is a shape to
    // adopt, and silently renaming the project's workflow to "simple" because
    // someone started from it would misreport what the project is running.
    setDraft({ ...entry.workflow, name: draft.name })
    // The entry's own positions if it has them, otherwise none — keeping the
    // previous workflow's coordinates would scatter a different set of states.
    setLayout(entry.layout ?? {})
    setSelection(null)
  }

  async function apply() {
    setApplying(true)
    try {
      const saved = await putProjectWorkflow(token, project, draft)
      // Layout is saved after the workflow, and its failure is not the
      // workflow's: positions are cosmetic, and a layout write that fails must
      // not make a successful workflow change look like it failed.
      void putLayout(token, project, view).catch(() => {})
      setApplied(true)
      onApplied(saved)
    } catch (e) {
      onError(e)
    } finally {
      setApplying(false)
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="text-[15px] font-[750] tracking-[-0.01em]">{labels.title}</h2>
          <p className="text-muted-foreground mt-1 max-w-prose text-[13px] leading-relaxed">
            {labels.subtitle}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {!readOnly && (
            <>
              <StartFromMenu library={library} onPick={startFrom} label={labels.startFrom} />
              <Button variant="secondary" size="sm" onClick={addState}>
                + {labels.addState}
              </Button>
            </>
          )}
        </div>
      </div>

      {readOnly && <p className="text-muted-foreground text-[12.5px]">{labels.readOnlyMsg}</p>}

      {/* Stacked, not side by side. The settings column is `max-w-3xl` because
          that is right for the forms above; putting a 260px inspector beside the
          canvas inside it left the graph about a third of the width it needs and
          clipped every workflow on first sight. The canvas takes the full column
          and scrolls; the inspector sits under it, where it has room for its
          hints. */}
      <div className="flex flex-col gap-4">
        <Canvas
          wf={draft}
          layout={view}
          selection={selection}
          onSelect={setSelection}
          onMove={move}
          onConnect={connect}
          readOnly={readOnly}
          labels={{
            initial: labels.canvasInitial,
            claimable: labels.canvasClaimable,
            terminal: labels.canvasTerminal,
            hint: labels.canvasHint,
          }}
        />
        <div className="border-border-soft min-w-0 rounded-xl border px-4 py-3">
          <Inspector
            wf={draft}
            selection={selection}
            onChange={setDraft}
            readOnly={readOnly}
            labels={labels}
          />
        </div>
      </div>

      {/* Preflight. Shown BEFORE Apply is reachable, because the server refuses
          this case and a 422 after the fact teaches nothing about which tickets
          are in the way. */}
      {stranded.length > 0 && (
        <div className="border-destructive/40 rounded-xl border border-l-2 px-4 py-3">
          <div className="text-[13px] font-[680]">{labels.blockedTitle}</div>
          <p className="text-muted-foreground mt-1 max-w-prose text-[12.5px] leading-relaxed">
            {labels.blockedBody}
          </p>
          <ul className="mt-2 flex flex-col gap-1">
            {stranded.map((s) => (
              <li key={s.state} className="flex flex-wrap items-center gap-2 text-[12.5px]">
                <Badge variant="destructive" className="font-mono">
                  {s.state}
                </Badge>
                <span className="text-muted-foreground">
                  {labels.blockedRow.replace('{n}', String(s.tickets))}
                </span>
                <a
                  className="text-primary underline-offset-4 hover:underline"
                  href={`/board?project=${encodeURIComponent(project)}`}
                >
                  {labels.openBoard}
                </a>
              </li>
            ))}
          </ul>
        </div>
      )}

      {verdict && !verdict.valid && stranded.length === 0 && (
        <div className="border-destructive/40 rounded-xl border border-l-2 px-4 py-3">
          <div className="text-[13px] font-[680]">{labels.problems}</div>
          <ul className="text-muted-foreground mt-1 flex list-disc flex-col gap-1 pl-5 text-[12.5px]">
            {verdict.problems.map((p) => (
              <li key={p}>{p}</li>
            ))}
          </ul>
        </div>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <Button
          onClick={() => void apply()}
          disabled={!canApply || applying}
          className={cn(!canApply && 'opacity-55')}
        >
          {applying ? labels.applying : labels.apply}
        </Button>
        {dirty && !readOnly && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setDraft(workflow)
              setSelection(null)
            }}
          >
            {labels.revert}
          </Button>
        )}
        {!readOnly && (
          <Button variant="secondary" size="sm" onClick={() => onSaveAs(draft, view)}>
            {labels.saveAs}
          </Button>
        )}
        <span className="grow" />
        <span className="text-muted-foreground text-[12px]">
          {checking
            ? labels.checking
            : applied && !dirty
              ? labels.applied
              : dirty && verdict?.valid
                ? labels.valid
                : ''}
        </span>
      </div>
    </div>
  )
}

/**
 * "Start from…" — the library as a picker.
 *
 * A native `<select>` rather than a Radix menu, the same call the project picker
 * makes: one control, correct on mobile for free, no portalled listbox weight.
 */
function StartFromMenu({
  library,
  onPick,
  label,
}: {
  library: WorkflowEntry[]
  onPick: (e: WorkflowEntry) => void
  label: string
}) {
  return (
    <select
      aria-label={label}
      title={label}
      value=""
      onChange={(e) => {
        const found = library.find((w) => w.id === e.target.value)
        if (found) onPick(found)
      }}
      className="bg-muted text-foreground border-border hover:border-ring max-w-55 cursor-pointer appearance-none rounded-lg border px-3 py-1.5 text-[13px] font-[650]"
    >
      <option value="">{label}</option>
      {library.map((w) => (
        <option key={w.id} value={w.id}>
          {w.name}
          {w.builtin ? ' ·' : ''}
        </option>
      ))}
    </select>
  )
}

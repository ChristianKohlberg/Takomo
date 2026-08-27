// The inspector: everything about the selected state or transition.
//
// The canvas carries structure — which states exist and what connects them —
// and this carries meaning. Splitting them that way is what keeps the canvas
// from needing text inputs inside SVG, which is a genuinely bad place for them.
//
// Requirements are checkboxes rather than free text because each one is a fixed
// vocabulary the server parses (`claim`, `scope:*`, `guard:*`). A text field
// accepts `scope:humam` and the server correctly refuses it — but only later, on
// Apply, by which point the reader has typed six more things.
import { Field } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'
import {
  CATEGORIES,
  HAS_LINK,
  REQ_CLAIM,
  REQ_HUMAN,
  REQ_NO_BLOCKERS,
  REQ_NO_CHILDREN,
  hasLinkKey,
  setHasLink,
  toggleRequirement,
  type Category,
  type WfState,
  type WfTransition,
  type WorkflowDoc,
} from '@/lib/workflows'
import type { CanvasSelection } from './Canvas'
import { Checkbox } from '@/components/ui/checkbox'

export interface InspectorLabels {
  nothing: string
  stateTitle: string
  transitionTitle: string
  id: string
  idHint: string
  category: string
  claimable: string
  claimableHint: string
  terminal: string
  terminalHint: string
  makeInitial: string
  isInitial: string
  deleteState: string
  deleteTransition: string
  requires: string
  reqClaim: string
  reqHuman: string
  reqNoChildren: string
  reqNoBlockers: string
  reqHasLink: string
  reqHasLinkHint: string
  linkKey: string
  from: string
  to: string
}

export interface InspectorProps {
  wf: WorkflowDoc
  selection: CanvasSelection | null
  onChange: (next: WorkflowDoc) => void
  readOnly?: boolean
  labels: InspectorLabels
}

export function Inspector({ wf, selection, onChange, readOnly, labels }: InspectorProps) {
  if (!selection) {
    return (
      <p className="text-muted-foreground border-border-soft rounded-xl border border-dashed px-4 py-6 text-center text-[13px]">
        {labels.nothing}
      </p>
    )
  }

  if (selection.kind === 'state') {
    const state = wf.states.find((s) => s.id === selection.key)
    if (!state) return null
    return (
      <StateInspector
        wf={wf}
        state={state}
        onChange={onChange}
        readOnly={readOnly}
        labels={labels}
      />
    )
  }

  const index = selection.key as number
  const transition = wf.transitions[index]
  if (!transition) return null
  return (
    <TransitionInspector
      wf={wf}
      transition={transition}
      index={index}
      onChange={onChange}
      readOnly={readOnly}
      labels={labels}
    />
  )
}

function StateInspector({
  wf,
  state,
  onChange,
  readOnly,
  labels,
}: {
  wf: WorkflowDoc
  state: WfState
  onChange: (next: WorkflowDoc) => void
  readOnly?: boolean
  labels: InspectorLabels
}) {
  const isInitial = wf.initial === state.id

  /**
   * Rename a state everywhere it is referenced.
   *
   * A state id is a foreign key in three places — every transition's `from` and
   * `to`, and the document's `initial`. Renaming only the state would leave
   * transitions pointing at an id that no longer exists, which the server
   * rejects; the rename has to carry.
   */
  function rename(next: string) {
    const id = next.trim()
    if (!id || wf.states.some((s) => s.id === id && s !== state)) return
    onChange({
      ...wf,
      initial: wf.initial === state.id ? id : wf.initial,
      states: wf.states.map((s) => (s.id === state.id ? { ...s, id } : s)),
      transitions: wf.transitions.map((t) => ({
        ...t,
        from: t.from === state.id ? id : t.from,
        to: t.to === state.id ? id : t.to,
      })),
    })
  }

  function patch(fields: Partial<WfState>) {
    onChange({
      ...wf,
      states: wf.states.map((s) => (s.id === state.id ? { ...s, ...fields } : s)),
    })
  }

  /** Deleting a state takes its edges with it — an edge to nowhere is invalid. */
  function remove() {
    onChange({
      ...wf,
      states: wf.states.filter((s) => s.id !== state.id),
      transitions: wf.transitions.filter((t) => t.from !== state.id && t.to !== state.id),
    })
  }

  return (
    <div className="flex flex-col gap-4">
      <h3 className="text-[13px] font-[750]">{labels.stateTitle}</h3>

      <Field label={labels.id} hint={labels.idHint}>
        {(id) => (
          <Input
            id={id}
            value={state.id}
            readOnly={readOnly}
            className="font-mono"
            onChange={(e) => rename(e.target.value)}
          />
        )}
      </Field>

      <Field label={labels.category}>
        {(id) => (
          <select
            id={id}
            value={state.category}
            disabled={readOnly}
            onChange={(e) => patch({ category: e.target.value as Category })}
            className="bg-muted text-foreground border-border hover:border-ring w-full cursor-pointer appearance-none rounded-lg border px-3 py-2 text-[13px]"
          >
            {CATEGORIES.map((c) => (
              <option key={c} value={c}>
                {c}
              </option>
            ))}
          </select>
        )}
      </Field>

      <Check
        checked={!!state.claimable}
        disabled={readOnly}
        label={labels.claimable}
        hint={labels.claimableHint}
        onChange={(v) => patch({ claimable: v })}
      />
      <Check
        checked={!!state.terminal}
        disabled={readOnly}
        label={labels.terminal}
        hint={labels.terminalHint}
        onChange={(v) => patch({ terminal: v })}
      />

      <div className="flex flex-wrap items-center gap-2">
        {isInitial ? (
          <span className="text-primary text-[12px] font-[650]">{labels.isInitial}</span>
        ) : (
          !readOnly && (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onChange({ ...wf, initial: state.id })}
            >
              {labels.makeInitial}
            </Button>
          )
        )}
        <span className="grow" />
        {/* The initial state has no delete: a workflow without one cannot be
            parsed at all, so removing it would produce a draft that can never
            validate until another is chosen. Choose first, then delete. */}
        {!readOnly && !isInitial && (
          <Button variant="destructive" size="sm" onClick={remove}>
            {labels.deleteState}
          </Button>
        )}
      </div>
    </div>
  )
}

function TransitionInspector({
  wf,
  transition,
  index,
  onChange,
  readOnly,
  labels,
}: {
  wf: WorkflowDoc
  transition: WfTransition
  index: number
  onChange: (next: WorkflowDoc) => void
  readOnly?: boolean
  labels: InspectorLabels
}) {
  const requires = transition.requires ?? []
  const linkKey = hasLinkKey(requires)

  function setRequires(next: string[]) {
    onChange({
      ...wf,
      transitions: wf.transitions.map((t, i) =>
        // An empty array is dropped rather than stored: the server omits
        // `requires` when it is empty, so keeping `[]` would make every
        // round-trip look like a change.
        i === index ? { ...t, requires: next.length ? next : undefined } : t,
      ),
    })
  }

  const toggle = (req: string, on: boolean) => setRequires(toggleRequirement(requires, req, on))

  return (
    <div className="flex flex-col gap-4">
      <h3 className="text-[13px] font-[750]">{labels.transitionTitle}</h3>

      <div className="text-[13px]">
        <span className="text-muted-foreground text-[10.5px] font-bold tracking-[0.05em] uppercase">
          {labels.from}
        </span>{' '}
        <span className="font-mono">{transition.from}</span>
        <span className="text-muted-foreground mx-2">→</span>
        <span className="text-muted-foreground text-[10.5px] font-bold tracking-[0.05em] uppercase">
          {labels.to}
        </span>{' '}
        <span className="font-mono">{transition.to}</span>
      </div>

      <div className="flex flex-col gap-1.5">
        <span className="text-muted-foreground text-[10.5px] font-bold tracking-[0.05em] uppercase">
          {labels.requires}
        </span>
        <Check
          checked={requires.includes(REQ_CLAIM)}
          disabled={readOnly}
          label="claim"
          hint={labels.reqClaim}
          onChange={(v) => toggle(REQ_CLAIM, v)}
        />
        <Check
          checked={requires.includes(REQ_HUMAN)}
          disabled={readOnly}
          label="scope:human"
          hint={labels.reqHuman}
          onChange={(v) => toggle(REQ_HUMAN, v)}
        />
        <Check
          checked={requires.includes(REQ_NO_CHILDREN)}
          disabled={readOnly}
          label="guard:no_open_children"
          hint={labels.reqNoChildren}
          onChange={(v) => toggle(REQ_NO_CHILDREN, v)}
        />
        <Check
          checked={requires.includes(REQ_NO_BLOCKERS)}
          disabled={readOnly}
          label="guard:no_open_blockers"
          hint={labels.reqNoBlockers}
          onChange={(v) => toggle(REQ_NO_BLOCKERS, v)}
        />
        <Check
          checked={linkKey !== null}
          disabled={readOnly}
          label={HAS_LINK + '…'}
          hint={labels.reqHasLink}
          onChange={(v) => setRequires(setHasLink(requires, v ? 'commit' : ''))}
        />
        {linkKey !== null && (
          <div className="pl-6">
            <Field label={labels.linkKey} hint={labels.reqHasLinkHint}>
              {(id) => (
                <Input
                  id={id}
                  value={linkKey}
                  readOnly={readOnly}
                  className="font-mono"
                  onChange={(e) => setRequires(setHasLink(requires, e.target.value))}
                />
              )}
            </Field>
          </div>
        )}
      </div>

      {!readOnly && (
        <div>
          <Button
            variant="destructive"
            size="sm"
            onClick={() =>
              onChange({ ...wf, transitions: wf.transitions.filter((_, i) => i !== index) })
            }
          >
            {labels.deleteTransition}
          </Button>
        </div>
      )}
    </div>
  )
}

function Check({
  checked,
  disabled,
  label,
  hint,
  onChange,
}: {
  checked: boolean
  disabled?: boolean
  label: string
  hint?: string
  onChange: (v: boolean) => void
}) {
  return (
    <label
      className={cn(
        'flex items-start gap-2.5 rounded-lg px-2 py-1.5',
        disabled ? 'opacity-60' : 'hover:bg-muted cursor-pointer',
      )}
    >
      <Checkbox
        className="mt-0.5"
        checked={checked}
        disabled={disabled}
        onCheckedChange={(e) => onChange(e === true)}
      />
      <span className="min-w-0">
        <span className="font-mono text-[12.5px] font-[650]">{label}</span>
        {hint && (
          <span className="text-muted-foreground block text-[11.5px] leading-snug">{hint}</span>
        )}
      </span>
    </label>
  )
}

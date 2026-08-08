import { EditableText } from '@takomo/web'

const ok = async () => undefined

/**
 * Edited in place and saved on blur. Retitling is the smallest possible change
 * and a modal for it would be heavier than the edit — so the title IS the field.
 */
export function EditableTitle() {
  return (
    <div style={{ maxWidth: 520 }}>
      <EditableText
        as="h1"
        value="Nested epics on the roadmap"
        editable
        required
        className="m-0 text-[22px] font-[740] tracking-[-0.02em] outline-none border-b border-dashed border-transparent"
        onCommit={ok}
      />
    </div>
  )
}

/** A summary, which may legitimately be emptied — so it is not `required`. */
export function EditableSummary() {
  return (
    <div style={{ maxWidth: 520 }}>
      <EditableText
        value="Whether /roadmap should present epic trees, and what it double-counts today."
        editable
        className="text-[14px] outline-none border-b border-dashed border-transparent"
        onCommit={ok}
      />
    </div>
  )
}

/**
 * Read-only: the same text with no affordance. This is what a token without the
 * `write` scope sees — the UI does not offer an edit it cannot perform.
 */
export function ReadOnly() {
  return (
    <div style={{ maxWidth: 520 }}>
      <EditableText
        as="h1"
        value="Nested epics on the roadmap"
        editable={false}
        className="m-0 text-[22px] font-[740] tracking-[-0.02em]"
        onCommit={ok}
      />
    </div>
  )
}

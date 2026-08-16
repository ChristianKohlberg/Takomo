import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import type { Entry } from '@/lib/initiatives'

export interface PaneEditorLabels {
  hint: string
  citesHeading: string
  citesHint: string
  noCites: string
  save: string
  cancel: string
  working: string
}

export interface PaneEditorProps {
  /** The pane's SOURCE text — local `[1]`-style marks, not the reader-facing numbering. */
  initialText: string
  /** The entries this pane cites, in the author's own order: index + 1 is the local mark. */
  cites: Entry[]
  busy: boolean
  labels: PaneEditorLabels
  onSave: (text: string) => void
  onCancel: () => void
}

/**
 * Write or revise one pane's prose.
 *
 * This edits the pane's SOURCE, where a citation is `[1]` meaning "the first id
 * in this pane's cites array" — not the global number the reader sees. Those two
 * numberings differ as soon as another pane cites anything, so the legend below
 * the box is not decoration: without it an author has no way to know what `[2]`
 * currently points at, and renumbering by hand would silently re-attribute a
 * sentence to the wrong source.
 *
 * Saving APPENDS a new `view`; it never edits the old one. The earlier wording
 * stays in the log, which is what keeps the argument that produced the current
 * text readable.
 */
export function PaneEditor({
  initialText,
  cites,
  busy,
  labels,
  onSave,
  onCancel,
}: PaneEditorProps) {
  // Uncontrolled from `initialText` on purpose: the entries refetch after every
  // append, and a controlled value would yank the text out from under a typist.
  const [text, setText] = useState(initialText)

  return (
    <form
      className="mt-2"
      onSubmit={(e) => {
        e.preventDefault()
        onSave(text)
      }}
    >
      <Textarea
        autoFocus
        rows={10}
        value={text}
        onChange={(e) => setText(e.target.value)}
        className="font-mono text-[13px] leading-[1.6]"
      />
      <p className="text-muted-foreground mt-1 mb-0 text-[11.5px]">{labels.hint}</p>

      <div className="border-border mt-2 rounded-md border p-2">
        <p className="text-muted-foreground m-0 text-[11px] font-bold tracking-[0.06em] uppercase">
          {labels.citesHeading}
        </p>
        {cites.length === 0 ? (
          <p className="text-muted-foreground mt-1 mb-0 text-[12px]">{labels.noCites}</p>
        ) : (
          <>
            <p className="text-muted-foreground mt-0.5 mb-1 text-[11.5px]">{labels.citesHint}</p>
            <ul className="m-0 list-none p-0">
              {cites.map((c, i) => (
                <li key={c.id} className="text-foreground py-0.5 text-[12px]">
                  <span className="bg-secondary text-secondary-foreground mr-1.5 rounded-[3px] px-1 font-mono text-[11px] font-bold">
                    [{i + 1}]
                  </span>
                  {c.kind} — {c.title || c.text?.slice(0, 70) || c.id}
                </li>
              ))}
            </ul>
          </>
        )}
      </div>

      <div className="mt-2 flex gap-1.5">
        <Button type="submit" size="sm" disabled={busy || !text.trim()}>
          {busy ? labels.working : labels.save}
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
          {labels.cancel}
        </Button>
      </div>
    </form>
  )
}

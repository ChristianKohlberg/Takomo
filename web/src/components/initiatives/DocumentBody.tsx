import { useCallback, useRef } from 'react'
import { CitationMark } from './CitationMark'
import { PaneEditor, type PaneEditorLabels } from './PaneEditor'
import { Button } from '@/components/ui/button'
import { makeAnchor, plainOffsetIn, type Anchor } from '@/lib/initiative-anchor'
import {
  citesOf,
  PANES,
  paneText,
  type Doc,
  type Pane,
  type PaneDoc,
  type Thread,
} from '@/lib/initiative-doc'
import { decorate, topSpan, type Span } from '@/lib/initiative-highlight'
import type { Entry } from '@/lib/initiatives'
import { cn } from '@/lib/utils'

export interface DocumentBodyLabels extends PaneEditorLabels {
  writePane: string
  revisePane: string
  citation: string
  uncited: string
  unwritten: string
  paneBusiness: string
  paneTechnical: string
  paneVerification: string
  orphanHeading: string
  orphanHint: string
  suggestionMark: string
}

export interface DocumentBodyProps {
  doc: Doc
  labels: DocumentBodyLabels
  /** The note or suggestion currently open in the side pane, highlighted brighter. */
  focusedSpan: string | null
  /** A live text selection was made. Null clears it — a click that selects nothing. */
  onSelect: (anchor: Anchor | null) => void
  onOpenSpan: (pane: Pane, id: string) => void
  onSelectSource: (entry: Entry, n: number) => void
  selectedSourceId: string | null
  /** Writing a pane is the one document action a reader can start without a highlight. */
  canWrite: boolean
  editing: Pane | null
  onStartEdit: (pane: Pane) => void
  onCancelEdit: () => void
  onSavePane: (pane: Pane, text: string) => void
  busy: boolean
}

/**
 * The document, rendered as ONE scrolling surface rather than three tabs.
 *
 * That is what makes highlighting work as a single gesture: a reader drags
 * across a sentence without first deciding which pane owns it, and the anchor
 * records the pane afterwards from where the selection landed. Tabs made the
 * pane a mode you had to be in; sections make it a place you scroll to.
 *
 * Selection is read on mouse/touch release rather than on every `selectionchange`
 * — mid-drag the selection is a moving target, and offering operations against a
 * range the reader has not finished choosing produces a menu that flickers.
 */
export function DocumentBody({
  doc,
  labels,
  focusedSpan,
  onSelect,
  onOpenSpan,
  onSelectSource,
  selectedSourceId,
  canWrite,
  editing,
  onStartEdit,
  onCancelEdit,
  onSavePane,
  busy,
}: DocumentBodyProps) {
  const rootRef = useRef<HTMLDivElement>(null)

  const readSelection = useCallback(() => {
    const sel = window.getSelection()
    const root = rootRef.current
    if (!sel || sel.isCollapsed || sel.rangeCount === 0 || !root) {
      onSelect(null)
      return
    }
    const range = sel.getRangeAt(0)
    // Both ends must be in ONE paragraph. A selection dragged across a paragraph
    // break has no single anchor, and silently keeping the first paragraph's part
    // of it would attach a note to less than the reader highlighted.
    const start = paragraphOf(range.startContainer, root)
    const end = paragraphOf(range.endContainer, root)
    if (!start || !end || start !== end) {
      onSelect(null)
      return
    }
    const pane = start.getAttribute('data-pane')
    const para = Number(start.getAttribute('data-para'))
    if (!pane || !Number.isInteger(para)) {
      onSelect(null)
      return
    }
    const from = plainOffsetIn(start, range.startContainer, range.startOffset)
    const to = plainOffsetIn(start, range.endContainer, range.endOffset)
    const paras = paneText(doc.panes[pane as Pane])
    onSelect(makeAnchor(paras, pane, para, from, to))
  }, [doc, onSelect])

  const paneLabel = (p: Pane): string =>
    p === 'business'
      ? labels.paneBusiness
      : p === 'technical'
        ? labels.paneTechnical
        : labels.paneVerification

  return (
    <div ref={rootRef} onMouseUp={readSelection} onTouchEnd={readSelection}>
      {PANES.map((p) => (
        <section key={p} className="mt-7 first:mt-4">
          <div className="flex items-baseline justify-between gap-2">
            <h2 className="text-muted-foreground m-0 mb-2 text-[11.5px] font-bold tracking-[0.08em] uppercase">
              {paneLabel(p)}
            </h2>
            {canWrite && editing !== p && (
              <Button variant="ghost" size="sm" onClick={() => onStartEdit(p)}>
                {doc.panes[p].entry ? labels.revisePane : labels.writePane}
              </Button>
            )}
          </div>
          {editing === p ? (
            <PaneEditor
              key={doc.panes[p].entry?.id ?? `empty:${p}`}
              initialText={doc.panes[p].entry?.text ?? ''}
              cites={citesEntriesOf(doc, p)}
              busy={busy}
              labels={labels}
              onSave={(text) => onSavePane(p, text)}
              onCancel={onCancelEdit}
            />
          ) : (
            <PaneSection
              pane={doc.panes[p]}
              labels={labels}
              focusedSpan={focusedSpan}
              onOpenSpan={(id) => onOpenSpan(p, id)}
              onSelectSource={onSelectSource}
              selectedSourceId={selectedSourceId}
            />
          )}
        </section>
      ))}
    </div>
  )
}

/**
 * The entries a pane cites, in the AUTHOR's local order — which is what `[1]`
 * means in the source, as opposed to the global number a reader sees.
 */
function citesEntriesOf(doc: Doc, pane: Pane): Entry[] {
  const entry = doc.panes[pane].entry
  if (!entry) return []
  const byId = new Map(doc.sources.map((e) => [e.id, e]))
  return citesOf(entry)
    .map((id) => byId.get(id))
    .filter((e): e is Entry => e !== undefined)
}

/** Walk up to the paragraph element a DOM node sits in, or null if outside one. */
function paragraphOf(node: Node, root: HTMLElement): HTMLElement | null {
  let el: Node | null = node
  while (el && el !== root) {
    if (el instanceof HTMLElement && el.hasAttribute('data-para')) return el
    el = el.parentNode
  }
  return null
}

function PaneSection({
  pane,
  labels,
  focusedSpan,
  onOpenSpan,
  onSelectSource,
  selectedSourceId,
}: {
  pane: PaneDoc
  labels: DocumentBodyLabels
  focusedSpan: string | null
  onOpenSpan: (id: string) => void
  onSelectSource: (entry: Entry, n: number) => void
  selectedSourceId: string | null
}) {
  if (pane.paragraphs.length === 0) {
    return <p className="text-muted-foreground m-0 text-[13.5px] italic">{labels.unwritten}</p>
  }

  // Threads and suggestions become spans on the paragraph they resolved to.
  // Orphans contribute nothing here — there is nowhere honest to draw them —
  // which is why they are listed separately below rather than dropped.
  const spansFor = (para: number): Span[] => [
    ...pane.threads
      .filter((t) => t.placed?.para === para)
      .map((t) => ({
        id: t.entry.id,
        kind: 'thread' as const,
        start: t.placed!.start,
        end: t.placed!.end,
        state: t.state,
      })),
    ...pane.pending
      .filter((a) => a.placed?.para === para)
      .map((a) => ({
        id: a.entry.id,
        kind: 'suggestion' as const,
        start: a.placed!.start,
        end: a.placed!.end,
      })),
  ]

  const orphans = pane.threads.filter((t) => t.orphaned)

  return (
    <>
      {pane.paragraphs.map((p, i) => (
        <p
          key={i}
          data-para={i}
          data-pane={pane.pane}
          className={cn(
            'text-foreground my-2.5 text-[14.5px] leading-[1.65]',
            // An assertion nobody sourced should LOOK like one.
            p.uncited && 'border-border border-l-2 pl-3',
          )}
        >
          {decorate(p.runs, spansFor(i)).map((piece, k) => {
            const top = topSpan(piece.spans)
            if (piece.kind === 'cite') {
              return (
                <CitationMark
                  key={k}
                  n={piece.cite}
                  label={labels.citation}
                  selected={selectedSourceId === piece.entry.id}
                  onSelect={() => onSelectSource(piece.entry, piece.cite)}
                />
              )
            }
            if (!top) return <span key={k}>{piece.text}</span>
            return (
              <mark
                key={k}
                onClick={() => onOpenSpan(top.id)}
                className={cn(
                  'cursor-pointer rounded-[2px] bg-transparent px-0 text-inherit',
                  top.kind === 'suggestion'
                    ? 'decoration-primary underline decoration-2 underline-offset-2'
                    : 'decoration-muted-foreground underline decoration-dotted underline-offset-2',
                  top.state === 'resolved' && 'opacity-60',
                  focusedSpan === top.id && 'bg-secondary text-secondary-foreground',
                )}
              >
                {piece.text}
              </mark>
            )
          })}
        </p>
      ))}

      {orphans.length > 0 && (
        <div className="border-border mt-4 border-t pt-3">
          <p className="text-muted-foreground m-0 text-[11.5px] font-bold tracking-[0.06em] uppercase">
            {labels.orphanHeading}
          </p>
          <p className="text-muted-foreground mt-0.5 mb-2 text-[12px]">{labels.orphanHint}</p>
          {orphans.map((t: Thread) => (
            <button
              key={t.entry.id}
              type="button"
              onClick={() => onOpenSpan(t.entry.id)}
              className="hover:bg-muted block w-full cursor-pointer rounded-md px-2 py-1.5 text-left"
            >
              <span className="text-muted-foreground line-through">{t.anchor?.quote}</span>
              <span className="text-foreground ml-2 text-[13px]">{t.entry.text}</span>
            </button>
          ))}
        </div>
      )}
    </>
  )
}

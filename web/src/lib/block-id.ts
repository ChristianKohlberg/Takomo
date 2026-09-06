// Stable ids on every top-level block.
//
// Ported from the doctest prototype's `shared/src/blockId.ts`, and it is the
// hinge the rest of KONZEPT hangs on rather than a convenience. An agent that
// returns *operations against block ids* — `replace blk_7f3a`, `insert_after
// blk_7f3a` — never touches a block it did not name, so a human editing three
// paragraphs away keeps their words. An agent that returns a document cannot
// make that promise no matter how it is prompted.
//
// Later stages address the same ids: a commitment marks a range, a check names
// the passage it verifies, a note anchors to the sentence that caused it.
//
// The id lives in a ProseMirror node attribute, which means it is part of the
// CRDT and merges like everything else. Two peers splitting the same paragraph
// concurrently can therefore both mint an id — see [`ensureBlockIds`], which
// resolves that by keeping the first occurrence.
import { DOMSerializer } from '@tiptap/pm/model'
import { Extension } from '@tiptap/react'
import { Plugin, PluginKey } from '@tiptap/pm/state'
import type { Node as PMNode } from '@tiptap/pm/model'

/** The block types that carry an id: every top-level structural node. */
const ID_TYPES = [
  'paragraph',
  'heading',
  'blockquote',
  'codeBlock',
  'bulletList',
  'orderedList',
  'horizontalRule',
  'table',
]

export function blockId(): string {
  // Six hex-ish characters, the same shape the prototype used. Collision odds
  // inside one document are negligible and `ensureBlockIds` repairs the rest.
  const rand = Math.random().toString(36).slice(2, 8)
  return `blk_${rand}`
}

/**
 * The `id` attribute, plus a plugin that fills it in and repairs duplicates.
 *
 * Duplicates are not hypothetical here. A CRDT merge can produce two blocks
 * carrying one id — two people splitting the same paragraph at the same moment
 * is the ordinary way — and a duplicate id is worse than a missing one, because
 * an agent op then addresses two places at once. The first occurrence keeps the
 * id; later ones are reissued.
 */
export const BlockId = Extension.create<{ canWrite: boolean }>({
  name: 'blockId',

  addOptions() {
    // Default false: minting is the exception, and a caller that forgets to say
    // so gets the safe behaviour rather than the writing one.
    return { canWrite: false }
  },

  addGlobalAttributes() {
    return [
      {
        types: ID_TYPES,
        attributes: {
          id: {
            default: null,
            parseHTML: (element: HTMLElement) => element.getAttribute('data-id'),
            renderHTML: (attributes: Record<string, unknown>) =>
              attributes.id ? { 'data-id': attributes.id } : {},
          },
        },
      },
    ]
  },

  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: new PluginKey('blockId'),
        appendTransaction: (_transactions, _oldState, newState) => {
          // A reader does not mint ids.
          //
          // `editable: false` does not stop an `appendTransaction`, so a
          // read-only viewer opening a section with an unnumbered block — or two
          // blocks sharing an id, which is the ORDINARY result of a concurrent
          // split — wrote to the shared document just by looking at it. Measured:
          // one Y.Doc update per mount. The server drops a read-only peer's
          // writes, so nothing persisted; what it left behind was worse in its
          // way — that reader's replica disagreeing with everyone else's about
          // block ids, so their highlights and diffs addressed blocks nobody
          // else had.
          if (!this.options.canWrite) return null
          const seen = new Set<string>()
          const fixes: { pos: number; id: string }[] = []

          newState.doc.descendants((node: PMNode, pos: number, parent: PMNode | null) => {
            // Top-level only: a paragraph inside a list item is part of that
            // block, not a block an agent addresses on its own.
            if (parent !== newState.doc) return false
            if (!ID_TYPES.includes(node.type.name)) return false
            const id = node.attrs.id as string | null
            if (!id || seen.has(id)) {
              const fresh = blockId()
              seen.add(fresh)
              fixes.push({ pos, id: fresh })
            } else {
              seen.add(id)
            }
            return false
          })

          if (!fixes.length) return null
          const tr = newState.tr
          for (const { pos, id } of fixes) {
            tr.setNodeAttribute(pos, 'id', id)
          }
          // Not undoable and not a user edit: this is bookkeeping the writer
          // never asked for, and putting it in the undo stack would make ⌘Z
          // appear to do nothing.
          tr.setMeta('addToHistory', false)
          return tr
        },
      }),
    ]
  },
})

/**
 * The document as markdown annotated with block ids — what an agent reads.
 *
 * ```
 * <!-- blk_7f3a -->
 * ## Pricing
 * Our current tiers are…
 * ```
 *
 * Deliberately a plain function over the ProseMirror doc rather than a method on
 * the editor: it is pure, so it is unit-testable without a browser, and the
 * agent-facing serialization is the one thing here that must not quietly change
 * shape.
 */
export function annotatedMarkdown(doc: PMNode): string {
  const out: string[] = []
  doc.forEach((node) => {
    const id = (node.attrs.id as string | null) ?? ''
    if (id) out.push(`<!-- ${id} -->`)
    out.push(nodeToMarkdown(node))
    out.push('')
  })
  return out.join('\n').trimEnd()
}

function nodeToMarkdown(node: PMNode): string {
  switch (node.type.name) {
    case 'table': {
      const container = document.createElement('div')
      container.appendChild(DOMSerializer.fromSchema(node.type.schema).serializeNode(node))
      container.querySelectorAll('[data-id]').forEach((el) => el.removeAttribute('data-id'))
      return container.innerHTML
    }
    case 'heading': {
      const level = Math.max(1, Math.min(Number(node.attrs.level) || 1, 6))
      return `${'#'.repeat(level)} ${node.textContent}`
    }
    case 'codeBlock':
      return '```' + (node.attrs.language ?? '') + '\n' + node.textContent + '\n```'
    case 'blockquote':
      return node.textContent
        .split('\n')
        .map((line) => `> ${line}`)
        .join('\n')
    case 'horizontalRule':
      return '---'
    case 'bulletList':
    case 'orderedList': {
      const ordered = node.type.name === 'orderedList'
      const lines: string[] = []
      node.forEach((item, _offset, index) => {
        lines.push(`${ordered ? `${index + 1}.` : '-'} ${item.textContent}`)
      })
      return lines.join('\n')
    }
    default:
      return node.textContent
  }
}

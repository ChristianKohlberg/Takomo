// Turning an agent's proposal into an edit — the browser's half of Stage 2.
//
// The server reads the document and stores the proposal; it deliberately does
// NOT construct nodes. Building ProseMirror content means knowing the editor's
// exact schema, and the editor is the only thing that does — Rust writing nodes
// it half-understands is how a shared document gets quietly corrupted. So the
// markdown→nodes step lives here, next to the schema it targets.
//
// The parser is deliberately small and closed. It accepts exactly the blocks
// StarterKit gives an id to, and anything it does not recognise becomes a
// paragraph rather than being dropped: a proposal that silently loses a line is
// worse than one that renders a line plainly, because only the second is visible
// to the person deciding.
import type { Node as PMNode, Schema } from '@tiptap/pm/model'
import type { Transaction } from '@tiptap/pm/state'

export type OpKind = 'replace' | 'insert_after' | 'delete'

export interface Op {
  op: OpKind
  id: string
  markdown?: string
  /** Why this one op, in the agent's words. Optional: the server stores it when
   *  an agent sends it, and a proposal is readable without it. */
  rationale?: string
}

export interface Proposal {
  id: string
  /**
   * The section this is about, when the document is a PLAN.
   *
   * A standalone document has no sections, so it is null there — which is why
   * this is read rather than required: one `proposals` map serves both, and the
   * plan view filters by it (`lib/plan-proposals.ts`).
   */
  node?: string | null
  status: 'pending' | 'accepted' | 'rejected'
  author: string
  instruction: string
  summary: string
  created_at: number
  skipped: string[]
  ops: Op[]
  decided_by?: string
  decided_at?: number
}

/** Parse a proposal record, or null if it is not one. Never throws. */
export function parseProposal(raw: unknown): Proposal | null {
  if (typeof raw !== 'string') return null
  try {
    const p = JSON.parse(raw) as Proposal
    if (!p || typeof p.id !== 'string' || !Array.isArray(p.ops)) return null
    return p
  } catch {
    return null
  }
}

/**
 * Markdown → ProseMirror nodes.
 *
 * Blocks are separated by blank lines, which is what makes a multi-block
 * `insert_after` expressible in one op.
 */
export function markdownToNodes(schema: Schema, markdown: string): PMNode[] {
  const out: PMNode[] = []
  const chunks = markdown.replace(/\r\n/g, '\n').split(/\n{2,}/)

  for (const chunk of chunks) {
    const block = chunk.trim()
    if (!block) continue

    // Fenced code first: its contents must not be read as any other block.
    const fence = block.match(/^```([^\n]*)\n([\s\S]*?)\n?```$/)
    if (fence && schema.nodes.codeBlock) {
      const [, language, code] = fence
      out.push(
        schema.nodes.codeBlock.create(
          { language: language?.trim() || null },
          code ? schema.text(code) : null,
        ),
      )
      continue
    }

    const heading = block.match(/^(#{1,6})\s+(.*)$/)
    if (heading && schema.nodes.heading) {
      const [, hashes, text] = heading
      out.push(
        schema.nodes.heading.create(
          { level: hashes!.length },
          text ? schema.text(text) : null,
        ),
      )
      continue
    }

    if (/^---+$/.test(block) && schema.nodes.horizontalRule) {
      out.push(schema.nodes.horizontalRule.create())
      continue
    }

    const lines = block.split('\n')

    if (lines.every((l) => /^>\s?/.test(l)) && schema.nodes.blockquote) {
      const text = lines.map((l) => l.replace(/^>\s?/, '')).join('\n')
      out.push(schema.nodes.blockquote.create(null, paragraph(schema, text)))
      continue
    }

    const bullets = lines.every((l) => /^[-*]\s+/.test(l))
    const ordered = lines.every((l) => /^\d+\.\s+/.test(l))
    if ((bullets || ordered) && schema.nodes.listItem) {
      const listType = bullets ? schema.nodes.bulletList : schema.nodes.orderedList
      if (listType) {
        const items = lines.map((l) =>
          schema.nodes.listItem!.create(null, paragraph(schema, l.replace(/^([-*]|\d+\.)\s+/, ''))),
        )
        out.push(listType.create(null, items))
        continue
      }
    }

    // Anything else, including a line the parser does not understand.
    out.push(paragraph(schema, lines.join(' ')))
  }

  // An op whose markdown parsed to nothing still has to produce a node, or
  // `replace` would silently delete the block it was meant to rewrite.
  if (!out.length) out.push(paragraph(schema, ''))
  return out
}

function paragraph(schema: Schema, text: string): PMNode {
  return schema.nodes.paragraph!.create(null, text ? schema.text(text) : null)
}

/** Where a block with this id currently sits, or null if it is gone. */
export function findBlock(doc: PMNode, id: string): { pos: number; node: PMNode } | null {
  let found: { pos: number; node: PMNode } | null = null
  doc.forEach((node, offset) => {
    if (!found && node.attrs.id === id) found = { pos: offset, node }
  })
  return found
}

/**
 * Apply a proposal's operations to a transaction.
 *
 * Positions are resolved through `tr.mapping` as it goes, which is what makes a
 * multi-op proposal correct: replacing the first block changes where the second
 * one starts, and an op batch computed against the original document would land
 * increasingly wrong.
 *
 * Ops whose block has disappeared since the proposal was made are **skipped and
 * reported**, not fatal — the same rule the server applies when validating.
 * Somebody deleting a paragraph must not make an otherwise good proposal
 * unacceptable.
 */
export function applyOps(
  tr: Transaction,
  schema: Schema,
  ops: readonly Op[],
): { applied: number; skipped: string[] } {
  const skipped: string[] = []
  let applied = 0
  // How much has already been inserted after each anchor in THIS batch.
  //
  // Every op re-finds its block in the current document, so two `insert_after`
  // ops naming the same block both landed immediately after it — and the second
  // therefore landed ABOVE the first. The reviewer accepted an ordered list and
  // got it backwards, with `applied: 2` and nothing skipped to hint at it. The
  // nodes a previous op inserted sit directly after the anchor, so stepping past
  // them keeps the batch in the order it was written.
  const insertedAfter = new Map<string, number>()

  for (const op of ops) {
    // Re-find against the CURRENT doc each time: earlier ops in this batch have
    // already moved things.
    const hit = findBlock(tr.doc, op.id)
    if (!hit) {
      skipped.push(`${op.op} ${op.id}: that block is no longer in the document`)
      continue
    }
    const { pos, node } = hit

    if (op.op === 'delete') {
      tr.delete(pos, pos + node.nodeSize)
      applied++
      continue
    }

    const nodes = markdownToNodes(schema, op.markdown ?? '')
    if (op.op === 'replace') {
      // Carry the block id onto the replacement, so the block a later op in the
      // same batch names still exists. Without it a `replace` followed by an
      // `insert_after` on the same block dropped the insert and reported "that
      // block is no longer in the document" — blaming a peer for a removal this
      // very accept had just performed.
      const kept = nodes.map((n, i) =>
        i === 0 && n.type.spec.attrs && 'id' in n.type.spec.attrs
          ? n.type.create({ ...n.attrs, id: op.id }, n.content, n.marks)
          : n,
      )
      tr.replaceWith(pos, pos + node.nodeSize, kept)
    } else {
      const already = insertedAfter.get(op.id) ?? 0
      tr.insert(pos + node.nodeSize + already, nodes)
      insertedAfter.set(
        op.id,
        already + nodes.reduce((total, n) => total + n.nodeSize, 0),
      )
    }
    applied++
  }

  return { applied, skipped }
}

/** The block ids a proposal touches — what the editor highlights. */
export function touchedBlocks(proposals: readonly Proposal[]): Set<string> {
  const out = new Set<string>()
  for (const p of proposals) {
    if (p.status !== 'pending') continue
    for (const op of p.ops) out.add(op.id)
  }
  return out
}

/** The current text of a block, for showing beside what is proposed. */
export function blockText(doc: PMNode, id: string): string | null {
  const hit = findBlock(doc, id)
  return hit ? hit.node.textContent : null
}

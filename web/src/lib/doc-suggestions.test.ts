import { describe, expect, it } from 'vitest'

import { suggestionsFor, type SuggestionLabels } from './doc-suggestions'

const t: SuggestionLabels = {
  tighten: 'Tighten',
  simpler: 'Simpler',
  asCommitment: 'As commitment',
  asCommitmentHint: 'checkable',
  findContradiction: 'Contradicts?',
  findContradictionHint: 'open question',
  asList: 'As list',
  expandSection: 'Write out',
  splitSection: 'Split',
  addMissingItems: 'Add items',
  whatIsMissing: 'Missing?',
  whatIsMissingHint: 'open questions',
  contradictions: 'Contradictions',
  summarize: 'Summarize',
  addHeadings: 'Headings',
  addHeadingsHint: 'structure only',
}

describe('suggestionsFor', () => {
  it('offers section actions on a heading, not paragraph actions', () => {
    const s = suggestionsFor({ blockKind: 'heading', hasSelection: false }, t)
    expect(s.map((x) => x.label)).toContain('Write out')
    expect(s.map((x) => x.label)).not.toContain('As list')
  })

  it('offers whole-document actions when the caret is nowhere', () => {
    const s = suggestionsFor({ blockKind: null, hasSelection: false }, t)
    expect(s.map((x) => x.label)).toEqual(['Missing?', 'Contradictions', 'Headings', 'Summarize'])
  })

  it('offers list actions on a list', () => {
    for (const kind of ['bulletList', 'orderedList']) {
      const s = suggestionsFor({ blockKind: kind, hasSelection: false }, t)
      expect(s[0]!.label).toBe('Add items')
    }
  })

  it('leads with tightening on a selection, because that is what people ask for', () => {
    const s = suggestionsFor({ blockKind: 'paragraph', hasSelection: true }, t)
    expect(s[0]!.label).toBe('Tighten')
  })

  // The instructions carry the model's guardrails, so they are part of the
  // contract rather than copy. Each of these was a specific failure mode.
  it('tells the model not to invent behaviour when phrasing a commitment', () => {
    const s = suggestionsFor({ blockKind: 'paragraph', hasSelection: false }, t)
    const commitment = s.find((x) => x.label === 'As commitment')!
    expect(commitment.instruction).toMatch(/Erfinde kein Verhalten/)
    expect(commitment.instruction).toMatch(/nachschauen/)
  })

  it('tells the model to leave open questions open rather than answering them', () => {
    const s = suggestionsFor({ blockKind: null, hasSelection: false }, t)
    expect(s.find((x) => x.label === 'Missing?')!.instruction).toMatch(/Beantworte sie nicht/)
  })

  it('tells the model to mark a contradiction rather than pick a side', () => {
    const s = suggestionsFor({ blockKind: 'paragraph', hasSelection: true }, t)
    expect(s.find((x) => x.label === 'Contradicts?')!.instruction).toMatch(
      /statt eine der\s+beiden Seiten zu ändern|statt eine der beiden Seiten zu ändern/,
    )
  })

  it('never returns an empty menu — ⌘K must not open onto nothing', () => {
    for (const kind of [null, 'paragraph', 'heading', 'bulletList', 'codeBlock']) {
      for (const sel of [true, false]) {
        expect(suggestionsFor({ blockKind: kind, hasSelection: sel }, t).length).toBeGreaterThan(0)
      }
    }
  })
})

import { describe, expect, it } from 'vitest'

import type { MapNode, Relationship } from './mindmap-doc'
import {
  appendAnswer,
  clampText,
  cutTarget,
  firstSentence,
  foldSummary,
  questionTarget,
  trustOf,
} from './mindmap-lens'

const node = (over: Partial<MapNode> & { id: string }): MapNode => ({
  parent: null,
  order: 'a0',
  title: over.id,
  notes: '',
  at: null,
  edge_label: '',
  kind: 'thought',
  origin: 'human',
  reviewed: false,
  icons: [],
  color: '',
  shape: '',
  attachments: [],
  promoted: null,
  created_by: '',
  created_at: 0,
  updated_at: 0,
  position: 0,
  ...over,
})

describe('firstSentence', () => {
  it('stops at the first terminator', () => {
    expect(firstSentence('Ship it Friday. Then tell the team.')).toBe('Ship it Friday.')
    expect(firstSentence('Can we ship it? Probably not.')).toBe('Can we ship it?')
    expect(firstSentence('Ship it! Now.')).toBe('Ship it!')
  })

  it('returns the whole thing when there is no terminator at all', () => {
    // The common case: a note is usually one unpunctuated fragment, and
    // answering "nothing" there would leave most nodes with no line of substance.
    expect(firstSentence('a rough idea about pricing')).toBe('a rough idea about pricing')
  })

  it('does not mistake an abbreviation for the end of a sentence', () => {
    expect(firstSentence('Ship it by Q3, e.g. after the migration. Then review.')).toBe(
      'Ship it by Q3, e.g. after the migration.',
    )
    expect(firstSentence('Ask Dr. Fell first. She owns it.')).toBe('Ask Dr. Fell first.')
    expect(firstSentence('J. Random wrote it. Ask him.')).toBe('J. Random wrote it.')
  })

  it('does not break inside a number', () => {
    expect(firstSentence('Latency is 3.5 seconds today. Too slow.')).toBe(
      'Latency is 3.5 seconds today.',
    )
  })

  it('gives an empty string for empty notes, never an ellipsis', () => {
    expect(firstSentence('')).toBe('')
    expect(firstSentence('   \n  ')).toBe('')
  })

  it('flattens whitespace so one line stays one line', () => {
    expect(firstSentence('two\nlines   of it')).toBe('two lines of it')
  })

  it('clamps a long first sentence on a word boundary', () => {
    const long = `${'word '.repeat(40)}end.`
    const out = firstSentence(long, 40)
    expect(out.length).toBeLessThanOrEqual(41)
    expect(out.endsWith('…')).toBe(true)
    expect(out).not.toContain('wor…')
  })
})

describe('clampText', () => {
  it('leaves text that fits alone', () => {
    expect(clampText('short', 10)).toBe('short')
  })

  it('cuts hard when there is no word boundary worth using', () => {
    expect(clampText('aaaaaaaaaaaaaaa', 5)).toBe('aaaaa…')
  })
})

describe('foldSummary', () => {
  const tree = [
    node({ id: 'root' }),
    node({ id: 'a', parent: 'root', title: 'Pricing' }),
    node({ id: 'b', parent: 'a', title: 'Per seat' }),
    node({ id: 'c', parent: 'a', title: 'Per project' }),
    node({ id: 'd', parent: 'b', title: 'Volume discount' }),
    node({ id: 'leaf', parent: 'root', title: 'Nothing under me' }),
  ]

  it('says how many went and names them in tree order', () => {
    expect(foldSummary(tree, 'a')).toEqual({
      count: 3,
      text: 'Per seat · Volume discount · Per project',
    })
  })

  it('has nothing to say about a leaf', () => {
    expect(foldSummary(tree, 'leaf')).toBeNull()
    expect(foldSummary(tree, 'missing')).toBeNull()
  })

  it('still counts everything when the titles had to be clamped', () => {
    // The count is of what went; the text is of what fitted. A clamped summary
    // that also under-reported the count would be worse than no summary.
    const summary = foldSummary(tree, 'a', 12)
    expect(summary?.count).toBe(3)
    expect(summary?.text.endsWith('…')).toBe(true)
    expect(summary!.text.length).toBeLessThanOrEqual(13)
  })

  it('skips an untitled thought rather than leaving a gap in the join', () => {
    const withBlank = [
      node({ id: 'p' }),
      node({ id: 'q', parent: 'p', title: '  ' }),
      node({ id: 'r', parent: 'p', title: 'Named' }),
    ]
    expect(foldSummary(withBlank, 'p')).toEqual({ count: 2, text: 'Named' })
  })
})

describe('trustOf', () => {
  it('calls a reviewed human node confirmed', () => {
    expect(trustOf({ origin: 'human', reviewed: true })).toBe('confirmed')
  })

  it('calls an unreviewed agent node machine-written', () => {
    expect(trustOf({ origin: 'agent', reviewed: false })).toBe('machine')
  })

  it('calls an unreviewed human thought an idea nobody has confirmed', () => {
    expect(trustOf({ origin: 'human', reviewed: false })).toBe('unverified')
  })

  it('confirms an agent node a person has since read and kept', () => {
    // `reviewed` decides, whoever wrote it. Leaving this as unconfirmed would
    // mark a node unchecked BECAUSE a machine wrote it, however many people had
    // since agreed with it — which is the one answer this lens must not give.
    expect(trustOf({ origin: 'agent', reviewed: true })).toBe('confirmed')
  })
})

describe('questionTarget', () => {
  const rel = (id: string, from: string, to: string): Relationship => ({ id, from, to, label: '' })

  it('finds the far end whichever way the relation points', () => {
    expect(questionTarget([rel('r1', 'q', 'a')], 'q')).toBe('a')
    expect(questionTarget([rel('r1', 'a', 'q')], 'q')).toBe('a')
  })

  it('is null when the question is about nothing in particular', () => {
    expect(questionTarget([], 'q')).toBeNull()
    expect(questionTarget([rel('r1', 'a', 'b')], 'q')).toBeNull()
  })

  it('takes the first relation, which is stable because they arrive sorted', () => {
    expect(questionTarget([rel('r1', 'q', 'a'), rel('r2', 'q', 'b')], 'q')).toBe('a')
  })
})

describe('appendAnswer', () => {
  it('appends rather than replacing', () => {
    expect(appendAnswer('What we knew.', 'What we now know.')).toBe(
      'What we knew.\n\nWhat we now know.',
    )
  })

  it('is just the answer when there were no notes', () => {
    expect(appendAnswer('', 'The answer.')).toBe('The answer.')
    expect(appendAnswer('   ', 'The answer.')).toBe('The answer.')
  })

  it('changes nothing for a blank answer', () => {
    expect(appendAnswer('Existing.', '   ')).toBe('Existing.')
  })
})

describe('cutTarget', () => {
  const tree = [
    node({ id: 'root', title: 'Root' }),
    node({ id: 'kid', parent: 'root', title: 'Kid' }),
    node({ id: 'loose', title: 'Loose' }),
  ]

  it('names both ends of the edge it would cut', () => {
    expect(cutTarget(tree, 'kid')).toEqual({
      child: { id: 'kid', title: 'Kid' },
      parentTitle: 'Root',
    })
  })

  it('refuses a node that hangs off nothing', () => {
    expect(cutTarget(tree, 'loose')).toBeNull()
    expect(cutTarget(tree, 'root')).toBeNull()
    expect(cutTarget(tree, 'gone')).toBeNull()
  })
})

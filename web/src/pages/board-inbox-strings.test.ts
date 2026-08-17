// takomo-2hk4 — board and inbox share nav labels but deliberately diverge on a
// handful of keys. Per-page `strings.ts` tables make that safe at compile time;
// this test makes the divergence explicit so nobody "unifies" them by accident.
import { describe, it, expect } from 'vitest'
import { STR as board } from './board/strings'
import { STR as inbox } from './inbox/strings'

const SHARED_DIVERGENT_KEYS = [
  'advisory',
  'allClear',
  'gateOpen',
  'groupEpic',
  'noneForTicket',
] as const

describe('board/inbox string tables', () => {
  it('share keys that intentionally differ — never merge these tables', () => {
    for (const key of SHARED_DIVERGENT_KEYS) {
      expect(board.en[key]).not.toBe(inbox.en[key])
      expect(board.de[key]).not.toBe(inbox.de[key])
    }
  })

  it('agree on keys that are genuinely the same label on both surfaces', () => {
    const same = ['board', 'inbox', 'cancel', 'description', 'mine', 'settings'] as const
    for (const key of same) {
      expect(board.en[key]).toBe(inbox.en[key])
    }
  })
})

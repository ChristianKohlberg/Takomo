import { describe, expect, it } from 'vitest'

import { resolveName, type NamingSession } from './mindmap-naming'

const fresh: NamingSession = { fresh: true, previous: 'New thought' }
const existing: NamingSession = { fresh: false, previous: 'Pricing' }

describe('resolveName', () => {
  it('renames when a name was typed and committed', () => {
    expect(resolveName(existing, { text: '  Pricing v2 ', cancelled: false })).toEqual({
      kind: 'rename',
      title: 'Pricing v2',
    })
  })

  it('removes a node that was never named when the caret is abandoned', () => {
    // Escape on a thought that only ever held its placeholder: nothing was
    // created, so nothing may be left behind.
    expect(resolveName(fresh, { text: 'New thought', cancelled: true })).toEqual({
      kind: 'discard',
    })
    // …and typing into it first does not make it a thought. It was abandoned.
    expect(resolveName(fresh, { text: 'half a th', cancelled: true })).toEqual({ kind: 'discard' })
  })

  it('restores a title that already existed rather than touching the document', () => {
    expect(resolveName(existing, { text: 'Pricing v2', cancelled: true })).toEqual({ kind: 'keep' })
  })

  it('treats emptying an existing title as a deletion, behind the usual questions', () => {
    expect(resolveName(existing, { text: '   ', cancelled: false })).toEqual({ kind: 'prune' })
  })

  it('drops a brand-new thought committed empty instead of asking to delete it', () => {
    // There is nothing to ask about: the gesture made a box and it was left blank.
    expect(resolveName(fresh, { text: '', cancelled: false })).toEqual({ kind: 'discard' })
  })

  it('writes nothing when the committed name is the one already there', () => {
    expect(resolveName(existing, { text: ' Pricing ', cancelled: false })).toEqual({ kind: 'keep' })
  })
})

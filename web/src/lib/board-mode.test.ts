import { describe, it, expect } from 'vitest'
import { hashKey, modeFor } from './board-mode'

describe('hashKey', () => {
  it('reads a key with or without the leading #', () => {
    expect(hashKey('#a=tka_abc', 'a')).toBe('tka_abc')
    expect(hashKey('a=tka_abc', 'a')).toBe('tka_abc')
  })

  it('reads one key out of several', () => {
    expect(hashKey('#project=demo&t=demo-1&a=tka_x', 'a')).toBe('tka_x')
    expect(hashKey('#project=demo&t=demo-1&a=tka_x', 't')).toBe('demo-1')
  })

  it('percent-decodes the value', () => {
    expect(hashKey('#t=demo%2F1', 't')).toBe('demo/1')
  })

  it('treats a malformed escape as absent rather than throwing', () => {
    // A bad fragment must not take the page down before it can show anything.
    expect(hashKey('#t=%E0%A4%A', 't')).toBe('')
  })

  it('returns "" for a missing key, an empty value, or a bare key', () => {
    expect(hashKey('#a=x', 's')).toBe('')
    expect(hashKey('#a=', 'a')).toBe('')
    expect(hashKey('#a', 'a')).toBe('')
    expect(hashKey('', 'a')).toBe('')
  })

  it('does not match a key that merely ends with the name', () => {
    expect(hashKey('#qa=x', 'a')).toBe('')
  })
})

describe('modeFor — the precedence that matters', () => {
  it('answer-grant wins over everything', () => {
    // Following an answer link while signed in must land on the question you
    // were sent, not on your own board.
    expect(modeFor('#a=tka_1')).toEqual({ kind: 'answer', token: 'tka_1' })
    expect(modeFor('#a=tka_1&s=tks_2')).toEqual({ kind: 'answer', token: 'tka_1' })
    expect(modeFor('#s=tks_2&a=tka_1')).toEqual({ kind: 'answer', token: 'tka_1' })
  })

  it('share wins over the plain board', () => {
    expect(modeFor('#s=tks_2')).toEqual({ kind: 'share', token: 'tks_2' })
    expect(modeFor('#s=tks_2&t=demo-1')).toEqual({ kind: 'share', token: 'tks_2' })
  })

  it('falls through to the board, carrying a deep-linked ticket', () => {
    expect(modeFor('')).toEqual({ kind: 'board' })
    expect(modeFor('#')).toEqual({ kind: 'board' })
    expect(modeFor('#t=demo-3l2j')).toEqual({ kind: 'board', ticket: 'demo-3l2j' })
  })

  it('ignores an empty grant rather than entering a mode with no token', () => {
    // `#a=` with nothing after it is a truncated link, not an instruction to
    // open an answer page that cannot authenticate.
    expect(modeFor('#a=')).toEqual({ kind: 'board' })
    expect(modeFor('#s=')).toEqual({ kind: 'board' })
  })
})

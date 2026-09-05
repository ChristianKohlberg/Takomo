import { describe, expect, it } from 'vitest'

import { draftsForDrop, extensionOf, isUrl, kindForFilename, nameForText } from './mindmap-attach'
import { MAX_ATTACHMENTS } from './mindmap-doc'

const gists = { file: 'A file that was dropped here.' }

describe('extensionOf', () => {
  it('takes the last extension, lowercased', () => {
    expect(extensionOf('Report.FINAL.PDF')).toBe('pdf')
    expect(extensionOf('/some/path/notes.md')).toBe('md')
    expect(extensionOf('C:\\Users\\me\\thing.CSV')).toBe('csv')
  })

  it('says a dotfile has none, and neither does a bare name', () => {
    expect(extensionOf('.gitignore')).toBe('')
    expect(extensionOf('Makefile')).toBe('')
    expect(extensionOf('trailing.')).toBe('')
  })
})

describe('kindForFilename', () => {
  it('reads a pdf as a pdf', () => {
    expect(kindForFilename('spec.pdf')).toBe('pdf')
  })

  it('reads markdown and csv as a table', () => {
    expect(kindForFilename('notes.md')).toBe('table')
    expect(kindForFilename('rows.csv')).toBe('table')
  })

  it('reads source as code', () => {
    for (const name of ['store.rs', 'App.tsx', 'main.py', 'schema.sql', 'config.yaml']) {
      expect(kindForFilename(name)).toBe('code')
    }
  })

  it('reads audio as audio', () => {
    expect(kindForFilename('standup.m4a')).toBe('audio')
    expect(kindForFilename('take-2.WAV')).toBe('audio')
  })

  it('reads an image as a link, because there is no image kind', () => {
    // Inventing a seventh kind here would write a value into the shared document
    // that every other reader would have to guess at. An image IS a pointer to a
    // picture living somewhere else.
    expect(kindForFilename('screenshot.png')).toBe('link')
    expect(kindForFilename('logo.svg')).toBe('link')
  })

  it('falls back to a link for anything it does not recognise', () => {
    expect(kindForFilename('archive.xyz')).toBe('link')
    expect(kindForFilename('Makefile')).toBe('link')
  })
})

describe('isUrl / nameForText', () => {
  it('recognises the schemes worth pointing at', () => {
    expect(isUrl('https://example.com')).toBe(true)
    expect(isUrl('  HTTP://example.com ')).toBe(true)
    expect(isUrl('mailto:someone@example.com')).toBe(true)
    expect(isUrl('just some words')).toBe(false)
    expect(isUrl('javascript:alert(1)')).toBe(false)
  })

  it('names a URL by its last segment, else its host', () => {
    expect(nameForText('https://example.com/specs/auth.md')).toBe('auth.md')
    expect(nameForText('https://example.com')).toBe('example.com')
    expect(nameForText('https://example.com/?q=1')).toBe('example.com')
  })

  it('names prose by its first line', () => {
    expect(nameForText('  the pricing question\nand more  ')).toBe('the pricing question')
  })
})

describe('draftsForDrop', () => {
  it('turns a dropped file into a pointer with no ref at all', () => {
    const { add, refused } = draftsForDrop({ files: [{ name: 'spec.pdf' }], text: '' }, 0, gists)
    expect(refused).toBe(0)
    expect(add).toEqual([{ kind: 'pdf', name: 'spec.pdf', gist: gists.file, ref: '' }])
  })

  it('turns a dropped URL into a link carrying the URL', () => {
    const { add } = draftsForDrop(
      { files: [], text: 'https://example.com/specs/auth.md' },
      0,
      gists,
    )
    expect(add).toEqual([
      { kind: 'link', name: 'auth.md', gist: '', ref: 'https://example.com/specs/auth.md' },
    ])
  })

  it('turns dropped prose into a link too, keeping the text', () => {
    const { add } = draftsForDrop({ files: [], text: '  ask legal about this  ' }, 0, gists)
    expect(add).toEqual([
      { kind: 'link', name: 'ask legal about this', gist: '', ref: 'ask legal about this' },
    ])
  })

  it('prefers the files when a browser offers both', () => {
    // A file drag also carries its path as text, and adding the same thing twice
    // is never what the gesture meant.
    const { add } = draftsForDrop(
      { files: [{ name: 'a.rs' }], text: 'file:///tmp/a.rs' },
      0,
      gists,
    )
    expect(add).toHaveLength(1)
    expect(add[0]?.kind).toBe('code')
  })

  it('adds one per file, in the order they were dropped', () => {
    const { add } = draftsForDrop(
      { files: [{ name: 'a.pdf' }, { name: 'b.md' }, { name: 'c.mp3' }], text: '' },
      0,
      gists,
    )
    expect(add.map((a) => a.kind)).toEqual(['pdf', 'table', 'audio'])
  })

  it('fills up to the cap and reports what did not fit', () => {
    const files = Array.from({ length: 4 }, (_, i) => ({ name: `f${i}.pdf` }))
    const { add, refused } = draftsForDrop({ files, text: '' }, MAX_ATTACHMENTS - 2, gists)
    expect(add).toHaveLength(2)
    expect(refused).toBe(2)
  })

  it('adds nothing and refuses everything on a full node', () => {
    const { add, refused } = draftsForDrop(
      { files: [{ name: 'a.pdf' }], text: '' },
      MAX_ATTACHMENTS,
      gists,
    )
    expect(add).toEqual([])
    expect(refused).toBe(1)
  })

  it('does nothing at all for an empty drop', () => {
    expect(draftsForDrop({ files: [], text: '   ' }, 0, gists)).toEqual({ add: [], refused: 0 })
  })
})

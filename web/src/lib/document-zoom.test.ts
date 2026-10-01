// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { correctZoomedResizeDrag, fitZoom, formatZoom, parseZoom, readZoom, stepZoom, writeZoom, zoomShortcut } from './document-zoom'

beforeEach(() => localStorage.clear())

const key = (init: Partial<KeyboardEvent>) => ({ key: '', code: '', ctrlKey: false, metaKey: false, altKey: false, isComposing: false, ...init })

describe('document zoom settings', () => {
  it('parses only presets and fit, defaulting to 100 %', () => {
    expect(parseZoom('1.25')).toBe(1.25)
    expect(parseZoom('fit')).toBe('fit')
    expect(parseZoom('1.3')).toBe(1)
    expect(parseZoom('abc')).toBe(1)
    expect(parseZoom(null)).toBe(1)
  })
  it('stores per project and stores the default as nothing', () => {
    writeZoom('a', 1.5); writeZoom('b', 'fit')
    expect(readZoom('a')).toBe(1.5)
    expect(readZoom('b')).toBe('fit')
    expect(readZoom('c')).toBe(1)
    writeZoom('a', 1)
    expect(localStorage.getItem('takomo:document-zoom:a')).toBeNull()
  })
  it('steps through the presets from whatever is in effect, clamped at both ends', () => {
    expect(stepZoom(1, 1)).toBe(1.25)
    expect(stepZoom(1, -1)).toBe(0.9)
    expect(stepZoom(1.32, 1)).toBe(1.5)
    expect(stepZoom(1.32, -1)).toBe(1.25)
    expect(stepZoom(2, 1)).toBe(2)
    expect(stepZoom(0.5, -1)).toBe(0.5)
  })
  it('fits the 840px measure to the available width, between 100 % and 200 %', () => {
    expect(fitZoom(1260)).toBe(1.5)
    expect(fitZoom(1111)).toBe(1.32)
    expect(fitZoom(600)).toBe(1)
    expect(fitZoom(5000)).toBe(2)
    expect(fitZoom(0)).toBe(1)
  })
  it('formats per locale', () => {
    expect(formatZoom(1.25, 'en')).toBe('125%')
    expect(formatZoom(0.9, 'de')).toBe('90 %')
  })
})

describe('zoom shortcuts', () => {
  it('claims Ctrl+Alt+Plus/Minus/0 by key and leaves plain Ctrl for browser zoom', () => {
    expect(zoomShortcut(key({ ctrlKey: true, altKey: true, key: '+', code: 'Equal' }))).toBe('in')
    expect(zoomShortcut(key({ ctrlKey: true, altKey: true, key: '=', code: 'Equal' }))).toBe('in')
    expect(zoomShortcut(key({ ctrlKey: true, altKey: true, key: '-', code: 'Minus' }))).toBe('out')
    expect(zoomShortcut(key({ ctrlKey: true, altKey: true, key: '0', code: 'Digit0' }))).toBe('reset')
    expect(zoomShortcut(key({ ctrlKey: true, key: '+', code: 'Equal' }))).toBeNull()
    expect(zoomShortcut(key({ ctrlKey: true, key: '0', code: 'Digit0' }))).toBeNull()
    expect(zoomShortcut(key({ altKey: true, key: '0', code: 'Digit0' }))).toBeNull()
  })
  it('never swallows an AltGr character (Ctrl+Alt on Windows)', () => {
    // German layout: AltGr+0 is "}", AltGr++ is "~".
    expect(zoomShortcut(key({ ctrlKey: true, altKey: true, key: '}', code: 'Digit0' }))).toBeNull()
    expect(zoomShortcut(key({ ctrlKey: true, altKey: true, key: '~', code: 'BracketRight' }))).toBeNull()
    expect(zoomShortcut(key({ ctrlKey: true, altKey: true, key: '+', code: 'NumpadAdd' }))).toBe('in')
  })
  it('reads the physical key for Cmd+Option, whose key Option has changed', () => {
    expect(zoomShortcut(key({ metaKey: true, altKey: true, key: '≠', code: 'Equal' }))).toBe('in')
    expect(zoomShortcut(key({ metaKey: true, altKey: true, key: '–', code: 'Minus' }))).toBe('out')
    expect(zoomShortcut(key({ metaKey: true, altKey: true, key: 'º', code: 'Digit0' }))).toBe('reset')
    expect(zoomShortcut(key({ metaKey: true, key: '=', code: 'Equal' }))).toBeNull()
    expect(zoomShortcut(key({ ctrlKey: true, altKey: true, key: '+', isComposing: true }))).toBeNull()
  })
})

describe('column resize under zoom', () => {
  let cleanup = () => {}
  afterEach(() => { cleanup(); document.body.replaceChildren() })
  function setup(zoom: number) {
    const col = document.createElement('div')
    const editor = document.createElement('div')
    editor.className = 'ProseMirror resize-cursor'
    const cell = document.createElement('td')
    cell.id = 'cell'
    const plain = document.createElement('p')
    plain.id = 'plain'
    editor.append(cell)
    col.append(editor, plain)
    document.body.append(col)
    cleanup = correctZoomedResizeDrag(col, () => zoom)
    const seen: number[] = []
    const record = (event: MouseEvent) => seen.push(event.clientX)
    window.addEventListener('mousemove', record)
    window.addEventListener('mouseup', record)
    const off = () => { window.removeEventListener('mousemove', record); window.removeEventListener('mouseup', record) }
    return { seen, off }
  }
  it('scales the pointer delta of a handle drag by the zoom, then stops', () => {
    const { seen, off } = setup(2)
    document.getElementById('cell')!.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, clientX: 100 }))
    document.getElementById('cell')!.dispatchEvent(new MouseEvent('mousemove', { bubbles: true, clientX: 140 }))
    document.getElementById('cell')!.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, clientX: 180 }))
    document.getElementById('cell')!.dispatchEvent(new MouseEvent('mousemove', { bubbles: true, clientX: 200 }))
    off()
    expect(seen).toEqual([120, 140, 200])
  })
  it('leaves events alone at 100 % and outside a resize handle', () => {
    const first = setup(1)
    document.getElementById('cell')!.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, clientX: 100 }))
    document.getElementById('cell')!.dispatchEvent(new MouseEvent('mousemove', { bubbles: true, clientX: 140 }))
    first.off(); cleanup()
    expect(first.seen).toEqual([140])
    const second = setup(2)
    document.getElementById('plain')!.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, clientX: 100 }))
    document.getElementById('plain')!.dispatchEvent(new MouseEvent('mousemove', { bubbles: true, clientX: 140 }))
    second.off()
    expect(second.seen).toEqual([140])
  })
})

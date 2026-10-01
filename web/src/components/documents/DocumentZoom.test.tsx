// @vitest-environment jsdom
import { useRef } from 'react'
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { DocumentZoomControl, DocumentZoomSteps, useDocumentZoom } from './DocumentZoom'

let resize: (() => void) | null = null
beforeEach(() => {
  localStorage.clear()
  resize = null
  vi.stubGlobal('ResizeObserver', class { constructor(fn: () => void) { resize = fn } observe() {} disconnect() {} })
})
afterEach(() => { cleanup(); vi.unstubAllGlobals() })

function View({ project = 'p', locale = 'en' as const, width = 1000 }: { project?: string; locale?: 'en' | 'de'; width?: number }) {
  const column = useRef<HTMLDivElement>(null)
  const zoom = useDocumentZoom(project, column)
  return <div>
    <div role="toolbar" aria-label="Document tools"><DocumentZoomControl zoom={zoom} locale={locale} /><DocumentZoomSteps zoom={zoom} locale={locale} /></div>
    <div ref={el => { column.current = el; if (el) Object.defineProperty(el, 'clientWidth', { value: width, configurable: true }) }} style={{ paddingLeft: '24px', paddingRight: '24px' }}>
      <div data-testid="page" className="document-page" style={zoom.style} />
    </div>
  </div>
}

const trigger = () => screen.getByRole('button', { name: /^Zoom:/ })
const openMenu = () => fireEvent.keyDown(trigger(), { key: 'ArrowDown' })
const page = () => screen.getByTestId('page').style.getPropertyValue('--document-zoom')

describe('document zoom control', () => {
  it('shows 100 % by default and offers the presets, fit width and reset in a menu', () => {
    render(<View />)
    expect(trigger().getAttribute('aria-label')).toBe('Zoom: 100%')
    expect(page()).toBe('1')
    openMenu()
    const items = screen.getAllByRole('menuitemradio').map(item => item.textContent)
    expect(items).toEqual(['50%', '75%', '90%', '100%', '125%', '150%', '200%', 'Fit width'])
    expect(screen.getByRole('menuitemradio', { name: '100%' }).getAttribute('aria-checked')).toBe('true')
    expect(screen.getByRole('menuitem', { name: /Reset/ }).getAttribute('data-disabled')).not.toBeNull()
  })

  it('applies a chosen preset to the page, announces it and remembers it per project', () => {
    const view = render(<View />)
    openMenu()
    fireEvent.click(screen.getByRole('menuitemradio', { name: '150%' }))
    expect(page()).toBe('1.5')
    expect(trigger().getAttribute('aria-label')).toBe('Zoom: 150%')
    expect(screen.getByRole('status').textContent).toBe('Zoom 150%')
    expect(localStorage.getItem('takomo:document-zoom:p')).toBe('1.5')
    view.rerender(<View project="other" />)
    expect(page()).toBe('1')
    view.rerender(<View project="p" />)
    expect(page()).toBe('1.5')
    cleanup()
    render(<View />)
    expect(page()).toBe('1.5')
  })

  it('resets to 100 % and forgets the preference', () => {
    localStorage.setItem('takomo:document-zoom:p', '0.75')
    render(<View locale="de" />)
    expect(trigger().getAttribute('aria-label')).toBe('Zoom: 75 %')
    openMenu()
    fireEvent.click(screen.getByRole('menuitem', { name: /Zurücksetzen/ }))
    expect(page()).toBe('1')
    expect(localStorage.getItem('takomo:document-zoom:p')).toBeNull()
  })

  it('steps with the −/+ buttons and clamps at the ends', () => {
    render(<View />)
    const [zoomIn] = screen.getAllByRole('button', { name: 'Zoom in' })
    for (let i = 0; i < 5; i++) fireEvent.click(zoomIn!)
    expect(page()).toBe('2')
    expect(zoomIn!.hasAttribute('disabled')).toBe(true)
    fireEvent.click(screen.getAllByRole('button', { name: 'Zoom out' })[1]!)
    expect(page()).toBe('1.5')
  })

  it('follows Ctrl+Alt+Plus/Minus/0 and leaves Ctrl+Plus to the browser', () => {
    render(<View />)
    const plain = new KeyboardEvent('keydown', { key: '+', code: 'Equal', ctrlKey: true, bubbles: true, cancelable: true })
    act(() => { window.dispatchEvent(plain) })
    expect(plain.defaultPrevented).toBe(false)
    expect(page()).toBe('1')
    const zoomIn = new KeyboardEvent('keydown', { key: '+', code: 'Equal', ctrlKey: true, altKey: true, bubbles: true, cancelable: true })
    act(() => { window.dispatchEvent(zoomIn) })
    expect(zoomIn.defaultPrevented).toBe(true)
    expect(page()).toBe('1.25')
    act(() => { fireEvent.keyDown(window, { key: '-', code: 'Minus', ctrlKey: true, altKey: true }) })
    act(() => { fireEvent.keyDown(window, { key: '-', code: 'Minus', ctrlKey: true, altKey: true }) })
    expect(page()).toBe('0.9')
    act(() => { fireEvent.keyDown(window, { key: '≠', code: 'Equal', metaKey: true, altKey: true }) })
    expect(page()).toBe('1')
    act(() => { fireEvent.keyDown(window, { key: '-', code: 'Minus', ctrlKey: true, altKey: true }) })
    act(() => { fireEvent.keyDown(window, { key: '0', code: 'Digit0', ctrlKey: true, altKey: true }) })
    expect(page()).toBe('1')
    expect(screen.getByRole('status').textContent).toBe('Zoom 100%')
  })

  it('fits the page measure to the column and follows its width', () => {
    const view = render(<View width={1308} />)
    openMenu()
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Fit width' }))
    // (1308 − 2 × 24) / 840 = 1.5
    expect(page()).toBe('1.5')
    expect(trigger().getAttribute('aria-label')).toBe('Zoom: Fit width (150%)')
    expect(localStorage.getItem('takomo:document-zoom:p')).toBe('fit')
    view.rerender(<View width={700} />)
    act(() => resize?.())
    expect(page()).toBe('1')
    view.rerender(<View width={4000} />)
    act(() => resize?.())
    expect(page()).toBe('2')
    // A step from fit leaves fit for the next preset.
    fireEvent.click(screen.getAllByRole('button', { name: 'Zoom out' })[0]!)
    expect(page()).toBe('1.5')
    expect(localStorage.getItem('takomo:document-zoom:p')).toBe('1.5')
  })
})

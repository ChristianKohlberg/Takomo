import { describe, expect, it } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import { CodeReferences } from './CodeReferences'

const binding = {
  file: 'web/src/lib/plan-url.test.ts',
  selector: 'keeps the selected section',
  proves: 'The link carries the section ID.',
  limits: 'Does not exercise browser navigation.',
  status: 'mapped-not-run',
}

describe('CodeReferences', () => {
  it('keeps source details collapsed and explains that a mapping is not an execution result', () => {
    render(<CodeReferences lang="en" metadata={{ specification: { bindings: [binding], bindings_source_commit: '803fabd' } }} />)
    const summary = screen.getByText('Code references (1)')
    const disclosure = summary.closest('details')!
    expect(disclosure.open).toBe(false)
    fireEvent.click(summary)
    expect(disclosure.open).toBe(true)
    expect(screen.getByText(binding.file)).toBeTruthy()
    expect(screen.getByText(binding.selector)).toBeTruthy()
    expect(screen.getByText(binding.proves)).toBeTruthy()
    expect(screen.getByText(binding.limits)).toBeTruthy()
    expect(screen.getByText('803fabd')).toBeTruthy()
    expect(screen.getByText(/They are not test results/)).toBeTruthy()
    fireEvent.click(summary)
    expect(disclosure.open).toBe(false)
  })

  it.each([undefined, null, 7, [], {}, { specification: null }, { specification: { bindings: [] } }])('omits the disclosure when references are absent: %j', metadata => {
    const { container } = render(<CodeReferences lang="en" metadata={metadata} />)
    expect(container.querySelector('details')).toBeNull()
  })

  it('preserves valid references and explains skipped malformed entries without crashing', () => {
    render(<CodeReferences lang="en" metadata={{ specification: { bindings: [null, { file: {} }, binding, { file: 'test.ts', selector: ' ' }] } }} />)
    expect(screen.getByText('Code references (1)')).toBeTruthy()
    expect(screen.getByRole('status').textContent).toContain('Some references could not be displayed')
    expect(screen.getByText(binding.selector)).toBeTruthy()
  })

  it('does not treat an invalid collection as coverage', () => {
    render(<CodeReferences lang="en" metadata={{ specification: { bindings: 'passed' } }} />)
    expect(screen.getByText('Code references (0)')).toBeTruthy()
    expect(screen.getByRole('status')).toBeTruthy()
    expect(screen.queryByText('passed')).toBeNull()
  })

  it('renders missing proof boundaries explicitly and supports German', () => {
    render(<CodeReferences lang="de" metadata={{ specification: { bindings: [{ file: 'test.ts', selector: 'works', proves: {}, limits: [] }] } }} />)
    expect(screen.getByText('Code-Referenzen (1)')).toBeTruthy()
    expect(screen.getByText('Nicht beschrieben.')).toBeTruthy()
    expect(screen.getByText('Keine Grenzen dokumentiert.')).toBeTruthy()
    expect(screen.getByText(/Sie sind keine Testergebnisse/)).toBeTruthy()
  })
})

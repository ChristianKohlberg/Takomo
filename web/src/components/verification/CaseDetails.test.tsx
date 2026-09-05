import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { CaseDetails } from './CaseDetails'

describe('CaseDetails', () => {
  it('presents ordered instructions and the expected result while keeping other data collapsed', () => {
    const { container } = render(<CaseDetails lang="en" assignment={{
      steps: ['Open the document.', 'Switch to the map.'], expected: 'The same section stays selected.',
      scenario_id: 'scenario-1', spec_revision: 'revision-2', environment: { browser: 'Chrome' },
    }} />)
    expect(Array.from(container.querySelectorAll('ol li'), item => item.textContent)).toEqual(['Open the document.', 'Switch to the map.'])
    expect(screen.getByText('Expected result')).toBeTruthy()
    expect(screen.getByText('The same section stays selected.')).toBeTruthy()
    const summary = screen.getByText('Parameters')
    const details = summary.closest('details')!
    expect(details.open).toBe(false)
    expect(JSON.parse(details.querySelector('pre')!.textContent!)).toEqual({ scenario_id: 'scenario-1', spec_revision: 'revision-2', environment: { browser: 'Chrome' } })
    fireEvent.click(summary)
    expect(details.open).toBe(true)
  })

  it('does not show an empty parameters disclosure when all fields are readable', () => {
    render(<CaseDetails lang="de" assignment={{ steps: ['Dokument öffnen.'], expected: 'Der Abschnitt bleibt ausgewählt.' }} />)
    expect(screen.getByText('Schritte')).toBeTruthy()
    expect(screen.getByText('Erwartetes Ergebnis')).toBeTruthy()
    expect(screen.queryByText('Parameter')).toBeNull()
  })

  it.each([null, undefined, {}])('handles absent instructions without inventing a result: %j', assignment => {
    const { container } = render(<CaseDetails lang="en" assignment={assignment} />)
    expect(container.textContent).toBe('')
  })

  it.each([
    { steps: ['First', 2], expected: { outcome: 'unknown' }, variant: 'edge' },
    { steps: [], expected: '' },
    { steps: [' '], expected: false },
    { browser: 'Firefox', count: 2 },
    ['legacy', 'parameters'], 'ordinary text', 42, false,
  ])('preserves ordinary or malformed assignments under Parameters: %j', assignment => {
    const { container } = render(<CaseDetails lang="en" assignment={assignment} />)
    expect(screen.queryByText('Steps')).toBeNull()
    expect(screen.queryByText('Expected result')).toBeNull()
    expect(JSON.parse(container.querySelector('pre')!.textContent!)).toEqual(assignment)
    expect(container.querySelector('details')!.open).toBe(false)
  })

  it('shows a valid expected result while preserving malformed steps for inspection', () => {
    const { container } = render(<CaseDetails lang="en" assignment={{ steps: 'open the map', expected: 'The section remains selected.' }} />)
    expect(screen.getByText('The section remains selected.')).toBeTruthy()
    expect(JSON.parse(container.querySelector('pre')!.textContent!)).toEqual({ steps: 'open the map' })
  })
})

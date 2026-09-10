import { render, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { describe, expect, it } from 'vitest'
import { PageCollection } from './PageCollection'

describe('Legacy directory', () => {
  it('keeps secondary pages reachable with project scope without an admin requirement', () => {
    render(<MemoryRouter><PageCollection lang="en" project="project / one" /></MemoryRouter>)
    for (const [name, route] of [['Agent queue', 'agent-queues'], ['Bugs', 'bugs'], ['Epics', 'epics'], ['Initiatives', 'initiatives'], ['Schedules', 'schedules'], ['Environments', 'environments']]) {
      expect(screen.getByRole('link', { name: new RegExp(name!) }).getAttribute('href')).toBe(`/${route}?project=project%20%2F%20one`)
    }
    expect(screen.getAllByRole('link')).toHaveLength(6)
  })
  it('localizes the collection and supports unscoped navigation', () => {
    render(<MemoryRouter><PageCollection lang="de" /></MemoryRouter>)
    expect(screen.getByRole('heading', { name: 'Ältere Arbeitsbereiche' })).toBeTruthy()
    expect(screen.getByRole('link', { name: /Initiativen/ }).getAttribute('href')).toBe('/initiatives')
  })
})

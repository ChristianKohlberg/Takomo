import { fireEvent, render, screen, within } from '@testing-library/react'
import { expect, it } from 'vitest'
import { CollaboratorPresence } from './CollaboratorPresence'

it('summarizes repeated identities while retaining session counts', () => {
  render(<CollaboratorPresence peers={['claude', 'claude', 'Ada']} lang="en" />)
  fireEvent.click(screen.getByRole('button', { name: 'Collaborators: 2' }))
  const rows = screen.getAllByRole('listitem')
  expect(rows).toHaveLength(2)
  expect(within(rows[0]!).getByText('claude')).toBeTruthy()
  expect(within(rows[0]!).getByText('2 sessions')).toBeTruthy()
  expect(within(rows[1]!).getByText('1 session')).toBeTruthy()
})

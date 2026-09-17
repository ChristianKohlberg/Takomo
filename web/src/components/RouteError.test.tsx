import { render, screen } from '@testing-library/react'
import { createMemoryRouter, RouterProvider } from 'react-router'
import { describe, expect, it, vi } from 'vitest'
import { RouteError } from './RouteError'

describe('route recovery', () => {
  it('replaces a failed lazy route with recovery actions instead of the router stack trace', async () => {
    const log = vi.spyOn(console, 'error').mockImplementation(() => {})
    try {
      const router = createMemoryRouter([{
        path: '/', errorElement: <RouteError />,
        lazy: async () => { throw new Error('Failed to fetch dynamically imported module: private details') },
      }])
      render(<RouterProvider router={router} />)
      expect(await screen.findByRole('heading', { name: 'This page could not be loaded.' })).toBeTruthy()
      expect(screen.getByRole('button', { name: 'Reload page' })).toBeTruthy()
      expect(screen.getByRole('link', { name: 'Go to board' }).getAttribute('href')).toBe('/board')
      expect(screen.queryByText(/private details|Hey developer|Unexpected Application Error/)).toBeNull()
      router.dispose()
    } finally { log.mockRestore() }
  })
})

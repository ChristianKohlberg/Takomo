import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { EmbeddingSettings } from './EmbeddingSettings'
const config = { provider: 'voyage', endpoint: 'https://api.voyageai.com/v1/embeddings', model: 'voyage-4-lite', dimensions: 1024, quiet_seconds: 60, max_wait_seconds: 300, configured: true }
afterEach(() => { cleanup(); vi.unstubAllGlobals() })
it('never fetches global credentials for project-scoped administrators', () => {
  const fetch = vi.fn(); vi.stubGlobal('fetch', fetch)
  render(<EmbeddingSettings token="test" locale="en" allowed={false} />)
  expect(fetch).not.toHaveBeenCalled()
  expect(screen.getByText(/Only unrestricted administrators/)).toBeTruthy()
})
it('retains an unchanged credential by omission, sends replacement write-only then clears input', async () => {
  const requests: Record<string, unknown>[] = []
  vi.stubGlobal('fetch', vi.fn((_url: string, init?: RequestInit) => {
    if (init?.method === 'PUT') requests.push(JSON.parse(init.body as string))
    return Promise.resolve(new Response(JSON.stringify(config)))
  }))
  render(<EmbeddingSettings token="test" locale="en" allowed />)
  const input = await screen.findByLabelText('API key') as HTMLInputElement
  expect(input.type).toBe('password'); expect(input.value).toBe('')
  fireEvent.click(screen.getByRole('button', { name: 'Save' }))
  await waitFor(() => expect(requests).toHaveLength(1))
  expect(requests[0]).not.toHaveProperty('api_key')
  expect(requests[0]).not.toHaveProperty('configured')
  await screen.findByText('Search settings saved.')
  fireEvent.change(input, { target: { value: 'test-secret' } })
  fireEvent.click(screen.getByRole('button', { name: 'Save' }))
  await waitFor(() => expect(requests).toHaveLength(2))
  expect(requests[1]?.api_key).toBe('test-secret')
  await waitFor(() => expect(input.value).toBe(''))
  fireEvent.change(screen.getByLabelText('Provider'), { target: { value: 'openai' } })
  expect(screen.getByText(/previous key will not be reused/)).toBeTruthy()
})

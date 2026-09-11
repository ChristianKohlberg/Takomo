import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { CodexSettings } from './CodexSettings'
import { api } from '@/lib/api'
vi.mock('@/lib/api', () => ({ api: vi.fn() }))
afterEach(() => { cleanup(); vi.resetAllMocks() })
const worker = { id: 'cc-one', service_id: 'worker', projects: ['ttt'], seen_at: Date.now(), busy: false, action: null, command_id: null, expires_at: null, report: { status: 'disconnected' } }
it('restricts the settings to unrestricted administrators without fetching account details', () => {
 render(<CodexSettings token="scoped" locale="en" allowed={false} />)
 expect(screen.getByText(/Only administrators/)).toBeTruthy()
 expect(api).not.toHaveBeenCalled()
})
it('starts device login and presents the code on the trusted OpenAI page', async () => {
 vi.mocked(api).mockResolvedValueOnce({ items: [worker] }).mockResolvedValueOnce({}).mockResolvedValue({ items: [{ ...worker, action: 'login', expires_at: Date.now()+60000, report: { status: 'login_pending', device: { verification_url: 'https://auth.openai.com/codex/device', user_code: 'ABCD-1234' } } }] })
 render(<CodexSettings token="admin" locale="en" allowed />)
 fireEvent.click(await screen.findByRole('button', { name: 'Connect ChatGPT / Codex' }))
 await screen.findByText('ABCD-1234')
 expect(screen.getByRole('link', { name: 'Sign in on OpenAI' }).getAttribute('href')).toBe('https://auth.openai.com/codex/device')
 await waitFor(() => expect(api).toHaveBeenCalledWith('admin', '/integrations/codex/cc-one', expect.objectContaining({ method: 'POST', body: expect.stringContaining('"action":"login"') })))
 expect(screen.getByRole('button', { name: 'Cancel request' })).toBeTruthy()
})
it('shows the connection account, assigned projects and reported quota', async () => {
 vi.mocked(api).mockResolvedValue({ items: [{ ...worker, report: { status: 'connected', account: { email: 'test@example.invalid', plan: 'plus', auth_mode: 'chatgpt' }, limits: { primary: { used_percent: 20, resets_at: null } } } }] })
 render(<CodexSettings token="admin" locale="en" allowed />)
 expect(await screen.findByText(/test@example.invalid/)).toBeTruthy()
 expect(screen.getByText('Projects: ttt')).toBeTruthy()
 expect(screen.getByText(/20% used/)).toBeTruthy()
 expect(screen.getByRole('button', { name: 'Disconnect' })).toBeTruthy()
})

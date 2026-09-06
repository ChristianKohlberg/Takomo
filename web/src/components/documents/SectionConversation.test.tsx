import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SectionConversation } from './SectionConversation'
import { getSectionConversation, sendSectionMessage, type SectionConversationView } from '@/lib/section-conversation'

vi.mock('@/lib/section-conversation', () => ({ getSectionConversation: vi.fn(), sendSectionMessage: vi.fn() }))
const get = vi.mocked(getSectionConversation)
const post = vi.mocked(sendSectionMessage)
const empty: SectionConversationView = { conversation: null, messages: [], jobs: [] }
const finished: SectionConversationView = {
  conversation: { id: 'conversation-1' },
  messages: [
    { id: 'm1', role: 'user', body: 'Challenge this section.', created_at: 1000, job_id: 'j1' },
    { id: 'm2', role: 'assistant', body: 'What happens **after expiry**?', created_at: 2000, job_id: 'j1' },
  ],
  jobs: [{ id: 'j1', status: 'completed', error: null, created_at: 1000 }],
}
const running: SectionConversationView = { ...finished, messages: finished.messages.slice(0, 1), jobs: [{ ...finished.jobs[0]!, status: 'running' }] }
const props = { token: 'token', map: 'map-1', node: 'node-1', lang: 'en' as const, canAsk: true }
const button = (name: string) => screen.getByRole('button', { name }) as HTMLButtonElement
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (value: unknown) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
async function open() {
  fireEvent.click(button('Grill this section'))
  await screen.findByRole('textbox', { name: 'Your reply or question' })
}

beforeEach(() => {
  sessionStorage.clear()
  vi.clearAllMocks()
  get.mockResolvedValue(empty)
  post.mockResolvedValue(running)
})
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks() })

describe('SectionConversation', () => {
  it('only starts a job after an explicit action; renders safe persisted replies after reopening', async () => {
    get.mockResolvedValue(finished)
    const first = render(<SectionConversation {...props} />)
    expect(get).not.toHaveBeenCalled()
    await open()
    expect(screen.getByText('after expiry').tagName).toBe('B')
    expect(post).not.toHaveBeenCalled()
    first.unmount()
    render(<SectionConversation {...props} />)
    await screen.findByText('after expiry')
    expect(button('Hide conversation')).toBeTruthy()
  })

  it('submits a follow-up once and disables further messages while the job runs', async () => {
    get.mockResolvedValue(finished)
    const pending = deferred<SectionConversationView>()
    post.mockReturnValue(pending.promise)
    render(<SectionConversation {...props} />)
    await open()
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Expiry deletes it.' } })
    fireEvent.click(button('Send'))
    expect(button('Sending…').disabled).toBe(true)
    expect(post).toHaveBeenCalledTimes(1)
    expect(post.mock.calls[0]!.slice(0, 4)).toEqual(['token', 'map-1', 'node-1', 'Expiry deletes it.'])
    get.mockResolvedValue(running)
    await act(async () => pending.resolve(running))
    await screen.findByText('Working…')
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).disabled).toBe(true)
  })

  it('polls active work and displays failure instead of leaving the section working forever', async () => {
    vi.useFakeTimers()
    sessionStorage.setItem('takomo.section-conversation.map-1.node-1', 'open')
    get.mockResolvedValueOnce(running).mockResolvedValue({
      ...running, jobs: [{ ...running.jobs[0]!, status: 'failed', error: 'Agent service disconnected.' }],
    })
    render(<SectionConversation {...props} />)
    await act(async () => {})
    expect(screen.getByText('Working…')).toBeTruthy()
    await act(async () => vi.advanceTimersByTimeAsync(1000))
    expect(screen.getByRole('alert').textContent).toContain('Agent service disconnected.')
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).disabled).toBe(false)
  })

  it('does not leak a late old section response into the newly selected section', async () => {
    const old = deferred<SectionConversationView>()
    get.mockReturnValueOnce(old.promise).mockResolvedValue(empty)
    const ui = render(<SectionConversation {...props} />)
    fireEvent.click(button('Grill this section'))
    ui.rerender(<SectionConversation {...props} node="node-2" />)
    await open()
    await act(async () => old.resolve(finished))
    expect(screen.queryByText('after expiry')).toBeNull()
    expect(get.mock.calls[0]![3]?.aborted).toBe(true)
  })

  it('retries uncertain delivery with the same idempotency key', async () => {
    post.mockRejectedValueOnce(new Error('Network lost')).mockResolvedValue(running)
    render(<SectionConversation {...props} />)
    await open()
    fireEvent.click(button('Start grilling'))
    await screen.findByRole('alert')
    expect(screen.getByRole('alert').textContent).toContain('Network lost')
    fireEvent.click(button('Retry'))
    await waitFor(() => expect(post).toHaveBeenCalledTimes(2))
    expect(post.mock.calls[0]!.slice(0, 5)).toEqual(post.mock.calls[1]!.slice(0, 5))
  })

  it('keeps a rejected submission visible and the draft editable after a successful refresh', async () => {
    post.mockRejectedValue(Object.assign(new Error('Conversation limit reached'), { status: 409 }))
    render(<SectionConversation {...props} />)
    await open()
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Another question' } })
    fireEvent.click(button('Send'))
    await screen.findByText('Conversation limit reached')
    await waitFor(() => expect(get).toHaveBeenCalledTimes(2))
    expect(screen.getByRole('alert').textContent).toContain('Conversation limit reached')
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toBe('Another question')
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).disabled).toBe(false)
  })

  it('counts UTF-8 bytes before allowing a message', async () => {
    render(<SectionConversation {...props} />)
    await open()
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'é'.repeat(4001) } })
    expect(screen.getByRole('alert').textContent).toContain('8,000 UTF-8 bytes')
    expect(button('Send').disabled).toBe(true)
    expect(post).not.toHaveBeenCalled()
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'é'.repeat(4000) } })
    expect(button('Send').disabled).toBe(false)
  })

  it('keeps history readable after the conversation turn limit', async () => {
    get.mockResolvedValue({ ...finished, turn_limit: 1 })
    render(<SectionConversation {...props} />)
    fireEvent.click(button('Grill this section'))
    await screen.findByText('after expiry')
    expect(screen.getByText(/reached its turn limit/)).toBeTruthy()
    expect(screen.queryByRole('textbox')).toBeNull()
    expect(screen.queryByRole('button', { name: 'Send' })).toBeNull()
  })

  it('discards a pending response when the viewer token changes', async () => {
    const old = deferred<SectionConversationView>()
    get.mockReturnValueOnce(old.promise).mockResolvedValue(empty)
    const ui = render(<SectionConversation {...props} />)
    fireEvent.click(button('Grill this section'))
    ui.rerender(<SectionConversation {...props} token="different-token" />)
    await screen.findByRole('textbox')
    await act(async () => old.resolve(finished))
    expect(screen.queryByText('after expiry')).toBeNull()
    expect(get.mock.calls[1]![0]).toBe('different-token')
  })

  it('follows new messages but preserves a reader scrolling up until they send a reply', async () => {
    vi.useFakeTimers()
    vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockReturnValue(1000)
    vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(420)
    sessionStorage.setItem('takomo.section-conversation.map-1.node-1', 'open')
    const next: SectionConversationView = { ...finished, messages: [
      ...finished.messages,
      { ...finished.messages[1]!, id: 'm3', body: 'Another detail to consider.' },
    ] }
    get.mockResolvedValueOnce(finished).mockResolvedValue(next)
    render(<SectionConversation {...props} />)
    await act(async () => {})
    const history = screen.getByRole('log')
    expect(history.scrollTop).toBe(1000)
    fireEvent.scroll(history, { target: { scrollTop: 200 } })
    await act(async () => vi.advanceTimersByTimeAsync(4000))
    expect(screen.getByText('Another detail to consider.')).toBeTruthy()
    expect(history.scrollTop).toBe(200)
    const replied: SectionConversationView = { ...next, messages: [
      ...next.messages,
      { ...finished.messages[0]!, id: 'm4', body: 'Here is my answer.' },
    ] }
    post.mockResolvedValue(replied)
    get.mockResolvedValue(replied)
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Here is my answer.' } })
    fireEvent.click(button('Send'))
    await act(async () => {})
    expect(history.scrollTop).toBe(1000)
  })

  it('ignores an in-flight poll that predates a submitted turn', async () => {
    vi.useFakeTimers()
    sessionStorage.setItem('takomo.section-conversation.map-1.node-1', 'open')
    const stale = deferred<SectionConversationView>()
    get.mockResolvedValueOnce(finished).mockReturnValueOnce(stale.promise).mockResolvedValue(running)
    render(<SectionConversation {...props} />)
    await act(async () => {})
    await act(async () => vi.advanceTimersByTimeAsync(4000))
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'My answer' } })
    fireEvent.click(button('Send'))
    await act(async () => {})
    expect(screen.getByText('Working…')).toBeTruthy()
    await act(async () => stale.resolve(finished))
    expect(screen.getByText('Working…')).toBeTruthy()
  })

  it('shows load failures and lets readers open history without offering an unauthorized action', async () => {
    get.mockRejectedValueOnce(new Error('Connection failed')).mockResolvedValue(finished)
    render(<SectionConversation {...props} canAsk={false} />)
    fireEvent.click(button('Grill this section'))
    await screen.findByText('Connection failed')
    fireEvent.click(button('Retry'))
    await screen.findByText('after expiry')
    expect(screen.queryByRole('textbox')).toBeNull()
    expect(screen.queryByRole('button', { name: 'Send' })).toBeNull()
  })
})

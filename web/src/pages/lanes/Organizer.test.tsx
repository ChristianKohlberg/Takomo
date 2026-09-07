import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Organizer } from './Organizer'
import * as api from '@/lib/lane-organizer'
vi.mock('@/lib/lane-organizer')
const group = { lane_id: null, title: 'Reliable editing', purpose: 'Preserve edits', context: 'Reuse recovery storage', readiness: 'needs_clarification' as const, reason: 'Clarify reconnect behavior before implementation', ticket_ids: ['t-1'] }
const job: api.OrganizerJob = { id: 'job-1', status: 'completed', error: null, created_at: '2026-09-07T12:00:00Z', accepted_at: null, proposal: { groups: [group], unassigned: [{ ticket_id: 't-2', reason: 'Needs a separate decision' }] }, snapshot: { tickets: [{ id: 't-1', title: 'Recover edits', body: 'Recover after reconnect', state: 'ready', links: { specification: 'https://example.test/spec' } }, { id: 't-2', title: 'Release branding', state: 'brief' }], lanes: [] } }
const empty: api.OrganizerView = { conversation_id: null, messages: [], jobs: [], total: 0, limit: 20 }
const proposal: api.OrganizerView = { ...empty, jobs: [job], total: 1 }
const queued: api.OrganizerView = { ...proposal, jobs: [{ ...job, status: 'queued', proposal: null }] }
const props = { token: 'secret', project: 'demo', lang: 'en' as const, canOrganize: true, onAuthError: vi.fn(), onAccepted: vi.fn() }
function mount(extra = {}) { return render(<Organizer {...props} {...extra} />) }
async function compose(text = 'Group recovery work') { fireEvent.click(await screen.findByRole('button', { name: 'Organize pending work' })); fireEvent.change(screen.getByLabelText('What should the agent consider?'), { target: { value: text } }); fireEvent.click(screen.getByRole('button', { name: 'Ask Codex' })) }
beforeEach(() => { vi.resetAllMocks(); vi.mocked(api.getOrganizer).mockResolvedValue(empty) })
afterEach(() => vi.useRealTimers())
describe('Lane organizer', () => {
  it('requests a proposal without applying it and shows queued service guidance', async () => {
    vi.mocked(api.requestOrganization).mockResolvedValue(queued); mount(); await waitFor(() => expect(api.getOrganizer).toHaveBeenCalled()); vi.mocked(api.getOrganizer).mockResolvedValue(queued); await compose()
    await waitFor(() => expect(api.requestOrganization).toHaveBeenCalledWith('secret', 'demo', { message: 'Group recovery work', request_id: expect.any(String) }))
    expect(api.acceptOrganization).not.toHaveBeenCalled(); expect(await screen.findByText('A configured agent service must pick up this request.')).toBeTruthy(); expect(screen.getByRole('link', { name: 'Open agent queue' })).toHaveProperty('pathname', '/agent-queues'); expect(screen.getByRole('button', { name: 'Organize pending work' })).toHaveProperty('disabled', true)
  })
  it('shows frozen ticket context, readiness and unassigned work before explicit acceptance', async () => {
    vi.mocked(api.getOrganizer).mockResolvedValue(proposal); mount(); await screen.findByRole('heading', { name: 'Reliable editing' })
    expect(screen.getByText('Needs clarification')).toBeTruthy(); expect(screen.getByText(group.reason)).toBeTruthy(); expect(screen.getByText('Recover after reconnect')).toBeTruthy(); expect(screen.getByText('https://example.test/spec')).toBeTruthy(); expect(screen.getByRole('link', { name: 'Release branding' })).toBeTruthy(); expect(api.acceptOrganization).not.toHaveBeenCalled()
    const applied = { ...proposal, jobs: [{ ...job, accepted_at: '2026-09-07T12:05:00Z' }] }; vi.mocked(api.acceptOrganization).mockResolvedValue(applied); vi.mocked(api.getOrganizer).mockResolvedValue(applied); fireEvent.click(screen.getByRole('button', { name: 'Accept all proposed changes' })); await waitFor(() => expect(api.acceptOrganization).toHaveBeenCalledWith('secret', 'demo', 'job-1')); expect(props.onAccepted).toHaveBeenCalledTimes(1); expect(await screen.findByText('Proposal applied')).toBeTruthy()
  })
  it('preserves proposals after a stale-snapshot rejection', async () => {
    vi.mocked(api.getOrganizer).mockResolvedValue(proposal); vi.mocked(api.acceptOrganization).mockRejectedValue(new Error('Work changed. Request a new proposal.')); mount(); fireEvent.click(await screen.findByRole('button', { name: 'Accept all proposed changes' })); expect(await screen.findByRole('alert')).toHaveProperty('textContent', 'Work changed. Request a new proposal.'); expect(screen.getByRole('heading', { name: 'Reliable editing' })).toBeTruthy(); expect(props.onAccepted).not.toHaveBeenCalled()
  })
  it('allows inspection without organizer write permission', async () => {
    vi.mocked(api.getOrganizer).mockResolvedValue(proposal); mount({ canOrganize: false }); expect(await screen.findByRole('button', { name: 'Accept all proposed changes' })).toHaveProperty('disabled', true); expect(screen.getByRole('button', { name: 'Organize pending work' })).toHaveProperty('disabled', true)
  })
  it('reuses the request ID after an ambiguous network failure', async () => {
    vi.mocked(api.requestOrganization).mockRejectedValueOnce(new Error('Network interrupted')).mockResolvedValue(queued); mount(); await compose(); await screen.findByRole('alert'); const first = vi.mocked(api.requestOrganization).mock.calls[0]![2]; fireEvent.click(screen.getByRole('button', { name: 'Ask Codex' })); await waitFor(() => expect(api.requestOrganization).toHaveBeenCalledTimes(2)); expect(vi.mocked(api.requestOrganization).mock.calls[1]![2]).toEqual(first)
  })
  it('shows failed preparation and permits a new request', async () => {
    vi.mocked(api.getOrganizer).mockResolvedValue({ ...proposal, jobs: [{ ...job, status: 'failed', error: 'Agent service unavailable', proposal: null }] }); mount(); expect(await screen.findByRole('alert')).toHaveProperty('textContent', expect.stringContaining('Agent service unavailable')); expect(screen.getByRole('button', { name: 'Organize pending work' })).toHaveProperty('disabled', false)
  })
  it('polls queued work into a reviewable proposal', async () => {
    vi.useFakeTimers(); vi.mocked(api.getOrganizer).mockResolvedValueOnce(queued).mockResolvedValue(proposal); mount(); await act(async () => {}); expect(screen.queryByRole('heading', { name: 'Reliable editing' })).toBeNull(); await act(async () => { await vi.advanceTimersByTimeAsync(3000) }); expect(screen.getByRole('heading', { name: 'Reliable editing' })).toBeTruthy()
  })
  it('discards old project responses and drafts when the scope changes', async () => {
    let resolveOld!: (value: api.OrganizerView) => void; vi.mocked(api.getOrganizer).mockImplementationOnce(() => new Promise(resolve => { resolveOld = resolve })).mockResolvedValue(empty); const { rerender } = mount(); rerender(<Organizer {...props} project="other" />); await act(async () => { resolveOld(proposal) }); expect(screen.queryByRole('heading', { name: 'Reliable editing' })).toBeNull(); await compose('Other project only'); await waitFor(() => expect(api.requestOrganization).toHaveBeenCalledWith('secret', 'other', expect.objectContaining({ message: 'Other project only' })))
  })
  it('does not let an older read overwrite a newly queued request', async () => {
    let resolveOld!: (value: api.OrganizerView) => void
    vi.mocked(api.getOrganizer).mockImplementationOnce(() => new Promise(resolve => { resolveOld = resolve })).mockResolvedValue(queued)
    vi.mocked(api.requestOrganization).mockResolvedValue(queued)
    mount(); await compose()
    await screen.findByText('A configured agent service must pick up this request.')
    await act(async () => { resolveOld(proposal) })
    expect(screen.queryByRole('heading', { name: 'Reliable editing' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Organize pending work' })).toHaveProperty('disabled', true)
  })
  it('shows existing-lane context for comparison and never enables acceptance for an accepted proposal', async () => {
    vi.mocked(api.getOrganizer).mockResolvedValue({ ...proposal, jobs: [{ ...job, accepted_at: '2026-09-07T12:05:00Z', proposal: { groups: [{ ...group, lane_id: 'wl-1' }], unassigned: [] }, snapshot: { ...job.snapshot, lanes: [{ id: 'wl-1', title: group.title, purpose: group.purpose, context: 'Earlier decision', tickets: [] }] } }] }); mount(); fireEvent.click(await screen.findByRole('button', { name: 'Review proposal' })); expect(screen.getByText('Existing lane')).toBeTruthy(); expect(screen.getByText('Earlier decision')).toBeTruthy(); expect(screen.queryByRole('button', { name: 'Accept all proposed changes' })).toBeNull()
  })
})

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { Workspace } from './Workspace'
import * as api from '@/lib/lanes'
import { listTickets } from '@/lib/board'
vi.mock('@/lib/lanes')
vi.mock('@/lib/board', () => ({ listTickets: vi.fn() }))
const ticket = { id: 't-1', title: 'Recover edits', state: 'ready' }
const lane: api.Lane = { id: 'ln-1', project: 'demo', title: 'Reliable editing', purpose: 'Keep work safe', context: 'Use existing persistence', conversation_ref: null, archived: false, tickets: [ticket], handoff_count: 0 }
const handoff: api.Handoff = { id: 'ho-1', lane: lane.id, kind: 'implementation', provider: 'codex', instructions: 'Implement recovery', ticket_ids: [ticket.id], target_revision: null, parent_handoff: null, status: 'draft', result: null, revision: null, conversation_ref: null, snapshot: { lane, tickets: [ticket] } }
function mount(extra = {}) { return render(<Workspace token="secret" project="demo" lang="en" canWrite canSend onAuthError={vi.fn()} {...extra} />) }
async function open() { fireEvent.click(await screen.findByRole('button', { name: /Reliable editing/ })); await screen.findByRole('heading', { name: 'Reliable editing' }) }
beforeEach(() => {
 vi.resetAllMocks(); vi.mocked(api.listLanes).mockResolvedValue({ items: [lane], total: 1, limit: 200 }); vi.mocked(api.getLane).mockResolvedValue(lane); vi.mocked(api.listHandoffs).mockResolvedValue({ items: [], total: 0, limit: 200 }); vi.mocked(listTickets).mockResolvedValue([{ ...ticket, project: 'demo' }, { id: 't-2', title: 'Offline status', state: 'brief', project: 'demo' }])
})
describe('Lanes workspace', () => {
 it('creates a draft without dispatching and sends only on explicit action', async () => {
 mount(); await open(); fireEvent.click(screen.getByRole('button', { name: 'Prepare a handoff' })); fireEvent.change(screen.getByLabelText('Instructions'), { target: { value: 'Implement recovery' } }); vi.mocked(api.createHandoff).mockResolvedValue(handoff); vi.mocked(api.listHandoffs).mockResolvedValue({ items: [handoff], total: 1, limit: 200 }); fireEvent.click(screen.getByRole('button', { name: 'Save draft' })); await waitFor(() => expect(api.createHandoff).toHaveBeenCalledWith('secret', lane.id, { kind: 'implementation', provider: 'codex', instructions: 'Implement recovery', ticket_ids: ['t-1'] })); expect(api.dispatchHandoff).not.toHaveBeenCalled(); fireEvent.click(await screen.findByRole('button', { name: 'Send to agent' })); await waitFor(() => expect(api.dispatchHandoff).toHaveBeenCalledWith('secret', handoff.id))
 })
 it('prepares revision-specific reviews of a completed implementation', async () => {
 vi.mocked(api.listHandoffs).mockResolvedValue({ items: [{ ...handoff, status: 'completed', revision: 'abc123' }], total: 1, limit: 200 }); mount(); await open(); fireEvent.click(screen.getByRole('button', { name: 'Request review' })); expect(screen.getByLabelText('Target revision')).toHaveProperty('value', 'abc123'); fireEvent.change(screen.getByLabelText('Instructions'), { target: { value: 'Check recovery behavior' } }); fireEvent.click(screen.getByRole('button', { name: 'Save draft' })); await waitFor(() => expect(api.createHandoff).toHaveBeenCalledWith('secret', lane.id, expect.objectContaining({ kind: 'review', parent_handoff: 'ho-1', target_revision: 'abc123' })))
 })
 it('returns review findings to correction instructions and preserves context edits on refresh', async () => {
 vi.mocked(api.listHandoffs).mockResolvedValue({ items: [{ ...handoff, kind: 'review', status: 'completed', result: 'Reconnect loses edits', target_revision: 'abc123' }], total: 1, limit: 200 }); mount(); await open(); fireEvent.click(screen.getByRole('button', { name: 'Prepare fixes' })); expect(screen.getByLabelText('Instructions')).toHaveProperty('value', 'Review ho-1 (abc123):\nReconnect loses edits'); fireEvent.change(screen.getByLabelText('Durable context'), { target: { value: 'My unsaved decision' } }); vi.mocked(api.getLane).mockResolvedValue({ ...lane, context: 'Agent preparation result' }); fireEvent.click(screen.getByRole('button', { name: 'Refresh' })); await waitFor(() => expect(api.getLane).toHaveBeenCalledTimes(2)); expect(screen.getByLabelText('Durable context')).toHaveProperty('value', 'My unsaved decision')
 })
 it('resumes context updates after saving an edit successfully', async () => {
 mount(); await open(); fireEvent.change(screen.getByLabelText('Durable context'), { target: { value: 'Saved decision' } }); vi.mocked(api.updateLane).mockResolvedValue({ ...lane, context: 'Saved decision' }); vi.mocked(api.getLane).mockResolvedValue({ ...lane, context: 'Saved decision' }); fireEvent.click(screen.getByRole('button', { name: 'Save' })); await waitFor(() => expect(api.updateLane).toHaveBeenCalled()); await waitFor(() => expect(screen.queryByRole('button', { name: 'Cancel' })).toBeNull()); vi.mocked(api.getLane).mockResolvedValue({ ...lane, context: 'New prepared context' }); fireEvent.click(screen.getByRole('button', { name: 'Refresh' })); await waitFor(() => expect(screen.getByLabelText('Durable context')).toHaveProperty('value', 'New prepared context'))
 })
 it('shows unapplied preparation results without implying the lane was updated', async () => {
 vi.mocked(api.listHandoffs).mockResolvedValue({ items: [{ ...handoff, kind: 'preparation', status: 'completed', context_applied: false, result: 'Suggested context' }], total: 1, limit: 200 }); mount(); await open(); expect(screen.getByText(/The lane changed during preparation/)).toBeTruthy(); expect(screen.getByText('Suggested context')).toBeTruthy()
 })
 it('adds an existing project ticket', async () => {
 mount(); await open(); fireEvent.change(screen.getByLabelText('Find a project ticket'), { target: { value: 'Offline' } }); fireEvent.click(await screen.findByRole('button', { name: 'Add ticket: Offline status' })); await waitFor(() => expect(api.setLaneTicket).toHaveBeenCalledWith('secret', lane.id, 't-2', true))
 })
 it('hides mutations for read-only projects', async () => {
 mount({ canWrite: false }); expect(screen.queryByRole('button', { name: 'New lane' })).toBeNull(); await open(); expect(screen.queryByRole('button', { name: 'Prepare a handoff' })).toBeNull(); expect(screen.getByLabelText('Durable context')).toHaveProperty('disabled', true)
 })
 it('requires human dispatch permission even when drafting is allowed', async () => {
 vi.mocked(api.listHandoffs).mockResolvedValue({ items: [handoff], total: 1, limit: 200 }); mount({ canSend: false }); await open(); expect(screen.getByRole('button', { name: 'Send to agent' })).toHaveProperty('disabled', true); expect(screen.getByRole('button', { name: 'Cancel' })).toHaveProperty('disabled', true)
 })
 it('reports load errors instead of an empty list and supports retry', async () => {
 vi.mocked(api.listLanes).mockRejectedValueOnce(new Error('Network unavailable')); mount(); expect(await screen.findByRole('alert')).toHaveProperty('textContent', expect.stringContaining('Network unavailable')); expect(screen.queryByText(/No lanes yet/)).toBeNull(); fireEvent.click(screen.getByRole('button', { name: 'Retry' })); await screen.findByRole('button', { name: /Reliable editing/ })
 })
})

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createMindmap, listMindmaps } from './mindmaps'
import { openSpecification } from './open-specification'

vi.mock('./mindmaps', () => ({ createMindmap: vi.fn(), listMindmaps: vi.fn() }))
const map = { id: 'map-1', project: 'new-project', title: 'New project', nodes: 0, status: 'open' as const, created_at: '' }
const page = (items: typeof map[]) => ({ items, total: items.length, limit: 1 })
beforeEach(() => vi.resetAllMocks())

describe('opening a specification', () => {
  it('opens an existing plan without changing it', async () => {
    vi.mocked(listMindmaps).mockResolvedValue(page([map]))
    expect(await openSpecification('token', map.project, 'Different title', true)).toBe(map)
    expect(createMindmap).not.toHaveBeenCalled()
  })
  it('provides an empty plan on the first writable visit without a creation dialog', async () => {
    vi.mocked(listMindmaps).mockResolvedValue(page([]))
    vi.mocked(createMindmap).mockResolvedValue({ mindmap: map })
    expect(await openSpecification('token', map.project, map.title, true)).toBe(map)
    expect(createMindmap).toHaveBeenCalledWith('token', { project: map.project, title: map.title })
  })
  it('does not attempt writes for a reader', async () => {
    vi.mocked(listMindmaps).mockResolvedValue(page([]))
    expect(await openSpecification('token', map.project, map.title, false)).toBeNull()
    expect(createMindmap).not.toHaveBeenCalled()
  })
  it('opens the winner when another tab creates the plan concurrently', async () => {
    vi.mocked(listMindmaps).mockResolvedValueOnce(page([])).mockResolvedValueOnce(page([map]))
    vi.mocked(createMindmap).mockRejectedValue({ code: 'mindmap.project_has_one' })
    expect(await openSpecification('token', map.project, map.title, true)).toBe(map)
  })
  it('reports permission and other creation failures instead of hiding them', async () => {
    vi.mocked(listMindmaps).mockResolvedValue(page([]))
    const error = { status: 403, code: 'auth.scope' }
    vi.mocked(createMindmap).mockRejectedValue(error)
    await expect(openSpecification('token', map.project, map.title, true)).rejects.toBe(error)
  })
})

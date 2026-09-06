import { afterEach, describe, expect, it, vi } from 'vitest'
import { mountMermaid } from './mermaid'

const { render, initialize } = vi.hoisted(() => ({ render: vi.fn(), initialize: vi.fn() }))
vi.mock('mermaid', () => ({ default: { render, initialize } }))
const tick = () => new Promise((resolve) => setTimeout(resolve, 300))
afterEach(() => { render.mockReset() })

describe('Mermaid rendering', () => {
  it('isolates SVG in an image and locks security configuration', async () => {
    render.mockResolvedValue({ svg: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 204 263"><script>alert(1)</script></svg>' })
    const host = document.createElement('div')
    const cancel = mountMermaid(host, 'flowchart TD\nA --> B')
    await tick()
    expect(host.querySelector('svg, script')).toBeNull()
    expect(host.querySelector('img')?.src).toMatch(/^data:image\/svg\+xml/)
    expect(host.querySelector('img')?.width).toBe(204)
    expect(host.querySelector('img')?.height).toBe(263)
    expect(initialize).toHaveBeenCalledWith(expect.objectContaining({
      securityLevel: 'strict', htmlLabels: false,
      secure: expect.arrayContaining(['securityLevel', 'secure', 'htmlLabels', 'suppressErrorRendering']),
    }))
    cancel()
  })

  it('ignores an older render that finishes after a source edit', async () => {
    let finish!: (result: { svg: string }) => void
    render.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve }))
    const host = document.createElement('div')
    const cancel = mountMermaid(host, 'old')
    await tick()
    cancel()
    render.mockResolvedValueOnce({ svg: '<svg>new</svg>' })
    const stop = mountMermaid(host, 'new')
    await tick()
    finish({ svg: '<svg>old</svg>' })
    await Promise.resolve()
    await Promise.resolve()
    expect(decodeURIComponent(host.querySelector('img')!.src)).toContain('<svg>new</svg>')
    stop()
  })

  it('shows a readable failure and permits retry', async () => {
    render.mockRejectedValueOnce(new Error('bad diagram'))
    const host = document.createElement('div')
    const cancel = mountMermaid(host, 'bad')
    await tick()
    expect(host.textContent).toContain('Unable to render')
    cancel()
    render.mockResolvedValueOnce({ svg: '<svg/>' })
    const stop = mountMermaid(host, 'valid')
    await tick()
    expect(host.querySelector('img')).not.toBeNull()
    stop()
  })

  it('debounces edits and cancels unmounted previews', async () => {
    const host = document.createElement('div')
    const cancel = mountMermaid(host, 'unused')
    cancel()
    await tick()
    expect(render).not.toHaveBeenCalled()
    mountMermaid(host, 'a'.repeat(50_001))
    expect(host.textContent).toContain('too large')
    expect(render).not.toHaveBeenCalled()
  })
})

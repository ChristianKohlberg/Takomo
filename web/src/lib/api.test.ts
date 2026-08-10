import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from './api'

// 204 is a null-body status: the Response constructor REJECTS a body for it,
// even `''`. Passing null is not a workaround — it is what the browser hands
// the client for a real 204, which is exactly the case under test.
function respond(status: number, body: string) {
  return new Response(status === 204 ? null : body, { status })
}

function mockFetch(r: Response) {
  const spy = vi.fn().mockResolvedValue(r)
  vi.stubGlobal('fetch', spy)
  return spy
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('api', () => {
  it('parses a JSON body', async () => {
    mockFetch(respond(200, JSON.stringify({ id: 'demo' })))
    await expect(api('tk_x', '/projects')).resolves.toEqual({ id: 'demo' })
  })

  // The regression. Every DELETE in this API answers 204, and `r.json()` throws
  // on an empty body — so a revoke that SUCCEEDED surfaced to the caller as a
  // failure, which skipped the refetch and left the revoked token on screen
  // looking live.
  it('returns undefined for 204 No Content instead of throwing', async () => {
    mockFetch(respond(204, ''))
    await expect(api('tk_x', '/tokens/tok_1', { method: 'DELETE' })).resolves.toBeUndefined()
  })

  it('returns undefined for a 200 with an empty body', async () => {
    mockFetch(respond(200, ''))
    await expect(api('tk_x', '/whatever')).resolves.toBeUndefined()
  })

  it('sends the bearer token', async () => {
    const spy = mockFetch(respond(200, '{}'))
    await api('tk_secret', '/whoami')
    const init = spy.mock.calls[0]?.[1] as RequestInit
    expect((init.headers as Record<string, string>)['Authorization']).toBe('Bearer tk_secret')
  })

  it('flags 401 as an auth failure', async () => {
    mockFetch(respond(401, ''))
    await expect(api('tk_x', '/tickets')).rejects.toMatchObject({ auth: true, status: 401 })
  })

  it('keeps a 403 as an ordinary error, message and code intact', async () => {
    // A 403 is an authentic token refused ONE operation, and the server says
    // which scope is missing and how to get it. Flagging it `auth` discarded
    // that body and signed the reader out: a `human` token answering an
    // expert-gated approve lost its inbox instead of being told why.
    mockFetch(
      respond(
        403,
        JSON.stringify({
          code: 'question.approve_expertise',
          message: 'Approving this needs a domain expert: expert:domain:billing.',
        }),
      ),
    )
    await expect(api('tk_x', '/questions/q-1/answer')).rejects.toMatchObject({
      status: 403,
      code: 'question.approve_expertise',
      message: 'Approving this needs a domain expert: expert:domain:billing.',
    })
    await expect(api('tk_x', '/questions/q-1/answer')).rejects.not.toMatchObject({ auth: true })
  })

  it('surfaces message + remedy from an error body, and keeps the code', async () => {
    mockFetch(
      respond(
        422,
        JSON.stringify({ code: 'validation.title', message: 'Title is required.', remedy: 'Send a title.' }),
      ),
    )
    await expect(api('tk_x', '/tickets', { method: 'POST' })).rejects.toMatchObject({
      code: 'validation.title',
      message: 'Title is required. — Send a title.',
    })
  })

  it('keeps a non-JSON error body verbatim', async () => {
    mockFetch(respond(502, '<html>bad gateway</html>'))
    await expect(api('tk_x', '/tickets')).rejects.toMatchObject({
      status: 502,
      message: '<html>bad gateway</html>',
    })
  })
})

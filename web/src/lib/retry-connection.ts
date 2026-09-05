/** Retry transient failures so a page opened during an outage recovers on return. */
export async function retryConnection<T>(request: () => Promise<T>, signal: AbortSignal): Promise<T> {
  while (!signal.aborted) {
    try { return await request() }
    catch (error) {
      const status = (error as { status?: number }).status
      if (signal.aborted || (status !== undefined && status < 500)) throw error
      await new Promise<void>((resolve) => {
        const done = () => { clearTimeout(timer); window.removeEventListener('online', done); signal.removeEventListener('abort', done); resolve() }
        const timer = setTimeout(done, 3000)
        window.addEventListener('online', done, { once: true })
        signal.addEventListener('abort', done, { once: true })
        if (signal.aborted) done()
      })
    }
  }
  throw new DOMException('Connection cancelled', 'AbortError')
}

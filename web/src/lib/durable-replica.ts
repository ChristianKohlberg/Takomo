import * as Y from 'yjs'

export type LocalSave = 'loading' | 'saving' | 'saved' | 'error'

/** Append updates, never overwrite another tab's unsent work with a snapshot. */
export function persistReplica(doc: Y.Doc, key: string, changed: (state: LocalSave) => void) {
  let db: IDBDatabase | undefined
  let stopped = false
  let pending = 0
  let failed = false
  let writes = 0
  const origin = {}
  const open = new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open(`takomo.replica.v1:${key}`, 1)
    request.onupgradeneeded = () => request.result.createObjectStore('updates', { autoIncrement: true })
    request.onerror = () => reject(request.error)
    request.onblocked = () => reject(new Error('Local draft storage is blocked by another tab.'))
    request.onsuccess = () => { db = request.result; resolve(db) }
  })
  const report = () => { if (!stopped) changed(failed ? 'error' : pending ? 'saving' : 'saved') }
  const ready = open.then(database => new Promise<void>((resolve, reject) => {
    const tx = database.transaction('updates', 'readonly')
    const request = tx.objectStore('updates').getAll()
    request.onsuccess = () => {
      if (stopped) return
      try { Y.transact(doc, () => { for (const update of request.result) Y.applyUpdate(doc, update, origin) }, origin) }
      catch (error) { reject(error) }
    }
    tx.oncomplete = () => { report(); resolve() }
    tx.onabort = () => reject(tx.error)
    tx.onerror = () => reject(tx.error)
  }))
  const write = (update: Uint8Array, source: unknown) => {
    if (source === origin || stopped) return
    pending++
    report()
    void open.then(database => new Promise<void>((resolve, reject) => {
      const tx = database.transaction('updates', 'readwrite')
      const store = tx.objectStore('updates')
      store.add(update)
      // Merge the committed log inside one transaction. Unlike a local snapshot,
      // this includes concurrent writes from other tabs and pending causal data.
      if (++writes % 128 === 0) {
        const all = store.getAll()
        all.onsuccess = () => {
          try { const merged = Y.mergeUpdates(all.result); store.clear(); store.add(merged) }
          catch { tx.abort() }
        }
      }
      tx.oncomplete = () => resolve()
      tx.onabort = () => reject(tx.error)
      tx.onerror = () => reject(tx.error)
    })).catch(() => { failed = true }).finally(() => { pending--; report() })
  }
  doc.on('update', write)
  void ready.catch(() => { failed = true; report() })
  return {
    ready,
    destroy() {
      stopped = true
      doc.off('update', write)
      // close() lets already-created transactions finish; queued open callbacks
      // must run first so navigation never cancels the last edit's transaction.
      void open.then(() => { setTimeout(() => db?.close(), 0) }).catch(() => {})
    },
  }
}

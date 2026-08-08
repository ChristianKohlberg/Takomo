// A minimal external store, read through React's built-in
// `useSyncExternalStore`.
//
// Why not component state: every page's core interaction is polling
// `GET /v1/events?since=<cursor>` and folding the result into a ticket index.
// Holding that in `useState` inside a page component is how the four pages
// forked their polling logic in the first place, and it re-renders the whole
// tree on every tick. A store keeps the poll loop testable without rendering
// anything, and lets cards subscribe to only what they read.
//
// No Redux, no Zustand: `useSyncExternalStore` is in React, and this is 40
// lines. A dependency here would inline into all four pages.
import { useSyncExternalStore } from 'react'

export interface Store<T> {
  get: () => T
  set: (next: T | ((prev: T) => T)) => void
  subscribe: (listener: () => void) => () => void
}

export function createStore<T>(initial: T): Store<T> {
  let value = initial
  const listeners = new Set<() => void>()
  return {
    get: () => value,
    set: (next) => {
      const resolved = typeof next === 'function' ? (next as (prev: T) => T)(value) : next
      if (Object.is(resolved, value)) return // no listener work for a no-op write
      value = resolved
      for (const l of listeners) l()
    },
    subscribe: (listener) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
  }
}

/** Subscribe a component to a store, optionally to one slice of it. */
export function useStore<T, S = T>(store: Store<T>, select?: (v: T) => S): S {
  return useSyncExternalStore(
    store.subscribe,
    () => (select ? select(store.get()) : (store.get() as unknown as S)),
    () => (select ? select(store.get()) : (store.get() as unknown as S)),
  )
}

import type { SearchStatus } from './hybrid-search'
import type { Locale } from './i18n'
export type EmbeddingState = 'loading' | 'current' | 'pending' | 'running' | 'unconfigured' | 'error' | 'stale'
export function embeddingState(status: SearchStatus | null, error: string, localPending: boolean, awaitingFreshStatus: boolean): EmbeddingState {
  if (error) return 'error'
  if (!status) return 'loading'
  if (!status.configured) return 'unconfigured'
  if (status.failed > 0 || status.last_error) return 'error'
  if (status.projection !== 'current') return 'stale'
  if (localPending) return 'pending'
  if (status.running > 0) return 'running'
  if (status.queued > 0 || status.pending > 0) return 'pending'
  if (awaitingFreshStatus) return 'loading'
  // Section totals and passage totals have different units. Only confirmed passage coverage is complete.
  if (status.passages_indexed === status.passages_total && Number.isFinite(status.passages_total)) return 'current'
  return 'stale'
}
export const EMBEDDING_LABELS: Record<Locale, Record<EmbeddingState, string>> = {
  en: { loading: 'Checking embeddings…', current: 'Embeddings current', pending: 'Embeddings pending', running: 'Embedding in progress', unconfigured: 'Embeddings not configured', error: 'Embedding error', stale: 'Embeddings out of date' },
  de: { loading: 'Embeddings werden geprüft…', current: 'Embeddings aktuell', pending: 'Embeddings ausstehend', running: 'Embedding läuft', unconfigured: 'Embeddings nicht konfiguriert', error: 'Embedding-Fehler', stale: 'Embeddings veraltet' },
}

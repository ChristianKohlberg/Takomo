// Display formatting, ported from initiatives.html. Pure, so it is finally
// testable — these three were inline in the page and covered by nothing.

/** Compact relative age: 45s, 12m, 3h, 9d. `—` for missing or unparseable. */
export function fmtAge(iso: string | null | undefined, now: number = Date.now()): string {
  if (!iso) return '—'
  const d = new Date(iso)
  if (isNaN(d.getTime())) return '—'
  const s = Math.max(0, (now - d.getTime()) / 1000)
  if (s < 60) return Math.floor(s) + 's'
  if (s < 3600) return Math.floor(s / 60) + 'm'
  if (s < 86400) return Math.floor(s / 3600) + 'h'
  return Math.floor(s / 86400) + 'd'
}

/**
 * The same buckets as `fmtAge`, from a duration the API already computed in
 * SECONDS (`held_for_seconds`, `idle_seconds`) rather than from a timestamp.
 *
 * Reusing `fmtAge` here would mean converting a server-computed duration back
 * into an instant against the browser's clock, which is how a claim held for
 * ten minutes reads as "3h" on a laptop with a skewed clock.
 */
export function fmtDuration(seconds: number | null | undefined): string {
  if (seconds == null || !isFinite(seconds)) return '—'
  const s = Math.max(0, seconds)
  if (s < 60) return Math.floor(s) + 's'
  if (s < 3600) return Math.floor(s / 60) + 'm'
  if (s < 86400) return Math.floor(s / 3600) + 'h'
  return Math.floor(s / 86400) + 'd'
}

/**
 * Bytes for a human. The API already sends `megabytes` on the rollup, but a
 * 300-byte note would read as "0 MB" — so small sizes get their real unit.
 */
export function fmtBytes(n: number | null | undefined): string {
  const v = n || 0
  if (v < 1024) return v + ' B'
  if (v < 1024 * 1024) return (v / 1024).toFixed(v < 10240 ? 1 : 0) + ' KB'
  return (v / (1024 * 1024)).toFixed(v < 10 * 1024 * 1024 ? 2 : 1) + ' MB'
}

/**
 * Unicode scalar count — the same measure Rust's `str.chars().count()` uses.
 *
 * JavaScript's `.length` is UTF-16 code units, so a style guide near the limit
 * can look legal in the counter while the server refuses it (or vice versa).
 */
export function charCount(s: string): number {
  let n = 0
  for (const _ of s) n++
  return n
}

/** "a, b , ,c" -> ["a","b","c"] — the comma-separated label/tag inputs. */
export function splitList(s: string | null | undefined): string[] {
  return String(s || '')
    .split(',')
    .map((x) => x.trim())
    .filter((x) => x.length > 0)
}

/**
 * `<input type="datetime-local">` yields "2026-07-01T09:00", which is NOT
 * RFC 3339 — the server refuses it rather than guessing at a zone. The offset is
 * added here, where the browser's own zone is known.
 *
 * Returns null for empty or unparseable input so the caller can omit the field
 * rather than send something the server will reject.
 */
export function localInputToRfc3339(value: string): string | null {
  const v = value.trim()
  if (!v) return null
  const d = new Date(v)
  if (isNaN(d.getTime())) return null
  return d.toISOString()
}

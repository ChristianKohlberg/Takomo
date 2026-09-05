// Fractional indexing: an order key that survives concurrent inserts.
//
// The twin of `src/fracdex.rs`. Both exist because order keys are assigned on
// both sides — the browser mints one every time somebody presses Enter, the
// server mints them in batches when an agent grows a branch — and a mindmap
// whose two writers disagreed about order would reshuffle itself depending on
// who was looking.
//
// `tests/fixtures/fracdex-vectors.json` is checked by both suites. If you change
// the algorithm here, the Rust test fails too, which is the point.
//
// The rules, both enforced by `isValid`:
//   - every character is a DIGITS digit, and DIGITS is ASCII-ascending, so plain
//     string comparison is numeric comparison;
//   - no key ends in the lowest digit — "0" and "" would name the same fraction,
//     and nothing can be inserted before "0".
//
// Ordering is total only once ties are broken, and the caller breaks them by
// node id: two peers that independently mint the same key still agree on order.

/** ASCII-ascending, so `<` on strings is `<` on fractions. */
export const DIGITS = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz'

const BASE = DIGITS.length

/** The lowest digit, pulled out once: indexing a string is `string | undefined`. */
const LOWEST = DIGITS[0] as string

/**
 * The longest key this module will accept.
 *
 * Mirrors `MAX_KEY_LEN` in `src/fracdex.rs`. A key grows by at most one digit
 * per insert into the same gap, so this is far above anything real; it is here
 * because keys arrive from peers, and `midpoint` recurses once per digit.
 */
export const MAX_KEY_LEN = 256

/**
 * Is this a key this module could have produced?
 *
 * Every order key read out of the shared document goes through here. A peer is
 * not a trusted writer — the sync socket carries whatever a client sends — and a
 * malformed key would make sibling order depend on who read it.
 */
export function isValid(key: string): boolean {
  if (key.length === 0) return false
  if (key.length > MAX_KEY_LEN) return false
  if (key.endsWith(LOWEST)) return false
  for (const ch of key) {
    if (DIGITS.indexOf(ch) === -1) return false
  }
  return true
}

/**
 * A key strictly between `a` and `b`. `null` means unbounded.
 *
 * If `a >= b` this appends after `a` rather than throwing. That ordering really
 * can occur — two peers can each hold a state the other has not seen — and an
 * exception in the middle of somebody's typing is a worse answer than a node
 * landing one place from where it was aimed.
 */
export function between(a: string | null, b: string | null): string {
  // A neighbour that is not a key this module could have produced is treated as
  // no neighbour at all — the same rule `src/fracdex.rs` applies, so both sides
  // mint the same key from the same ring even when the ring holds junk.
  if (a !== null && !isValid(a)) a = null
  if (b !== null && !isValid(b)) b = null
  if (a === null && b === null) return midpoint('', null)
  if (b === null) return midpoint(a as string, null)
  if (a === null) return midpoint('', b)
  return a < b ? midpoint(a, b) : midpoint(a, null)
}

/** The first key in an empty ring. */
export function first(): string {
  return between(null, null)
}

/** `n` keys in ascending order, for seeding a ring in one pass. */
export function sequence(n: number): string[] {
  const out: string[] = []
  let prev: string | null = null
  for (let i = 0; i < n; i += 1) {
    prev = between(prev, null)
    out.push(prev)
  }
  return out
}

/** The recursive core. Precondition: `a < b` when `b` is not null. */
function midpoint(a: string, b: string | null): string {
  if (b !== null) {
    let common = 0
    while (common < a.length && common < b.length && a[common] === b[common]) common += 1
    if (common > 0) {
      return a.slice(0, common) + midpoint(a.slice(common), b.slice(common))
    }
  }

  // An empty `a` reads as the fraction 0; an absent `b` reads as 1.
  const da = a.length > 0 ? DIGITS.indexOf(a[0] as string) : 0
  const db = b !== null ? DIGITS.indexOf(b[0] as string) : BASE

  // Room between the leading digits: take the middle one and stop.
  if (db > da + 1) {
    return DIGITS[Math.floor((da + db) / 2)] as string
  }

  // Adjacent digits, so the answer is one digit longer. Descend where there is room.
  if (b !== null && b.length > 1) {
    return (b[0] as string) + midpoint('', b.slice(1))
  }

  return (DIGITS[da] as string) + midpoint(a.length > 0 ? a.slice(1) : '', null)
}

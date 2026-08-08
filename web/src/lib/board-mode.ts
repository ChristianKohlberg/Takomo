// Which of the board's three modes a URL asks for.
//
// `/board` is one route serving three audiences, and the two credential-bearing
// ones arrive by fragment:
//
//   #a=tka_…   an outside expert, sent a single-use link to answer ONE question
//   #s=tks_…   a read-only share of a project or a subtree
//   (neither)  the normal board, on a `tk_` token from localStorage
//
// The fragment WINS over a stored token, and that ordering is the whole point:
// someone who follows an answer link while signed in here must land on the
// question they were sent, not on their own board. Answer-grant beats share for
// the same reason — it is the more specific instruction.
//
// Fragments never reach the server, which is what lets these tokens ride in a
// URL at all: they are readable by the page and by nothing in the request log.

export type BoardMode =
  | { kind: 'answer'; token: string }
  | { kind: 'share'; token: string }
  | { kind: 'board'; ticket?: string }

/** Read one `key=value` out of a location fragment. Last occurrence wins. */
export function hashKey(hash: string, key: string): string {
  let h = hash || ''
  if (h.charAt(0) === '#') h = h.slice(1)
  let out = ''
  for (const part of h.split('&')) {
    const i = part.indexOf('=')
    if (i > 0 && part.slice(0, i) === key) {
      try {
        out = decodeURIComponent(part.slice(i + 1))
      } catch {
        // A malformed escape must not take the page down — treat it as absent.
        out = ''
      }
    }
  }
  return out
}

export function modeFor(hash: string): BoardMode {
  const answer = hashKey(hash, 'a')
  if (answer) return { kind: 'answer', token: answer }
  const share = hashKey(hash, 's')
  if (share) return { kind: 'share', token: share }
  const ticket = hashKey(hash, 't')
  return ticket ? { kind: 'board', ticket } : { kind: 'board' }
}

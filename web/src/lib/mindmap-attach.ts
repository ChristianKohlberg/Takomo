// What a dropped thing becomes.
//
// An attachment here is a POINTER, never bytes — the rule the whole feature is
// built around, because bytes in a CRDT update log are bytes every peer replays
// on join. So dropping a file onto a node cannot upload it. What it can do is
// record that the file exists, what it is called, and roughly what kind of thing
// it is, and say plainly that the file itself still lives wherever it was
// dragged from.
//
// That is the entire reason this module is pure and tested: the inference is the
// part that decides what a person sees after a gesture they cannot undo by
// dragging again, and jsdom cannot prove anything about the drop itself.
import {
  MAX_ATTACHMENTS,
  MAX_ATTACHMENT_NAME,
  MAX_ATTACHMENT_REF,
  type Attachment,
  type AttachmentKind,
} from './mindmap-doc'

/** An attachment before the document mints its id. Structurally the same shape
 *  `mindmap-crdt`'s `AttachmentDraft` is, without importing Yjs to say so. */
export type AttachDraft = Omit<Attachment, 'id'>

/**
 * Extensions, grouped by the kind they mean.
 *
 * There is deliberately no `image` kind — the six kinds are the document's, not
 * this module's, and inventing a seventh here would write a value into shared
 * state that every other reader would have to guess at. An image is a `link`,
 * which is exactly what it is: a pointer to a picture that lives elsewhere.
 */
const BY_KIND: [AttachmentKind, readonly string[]][] = [
  ['pdf', ['pdf']],
  ['table', ['md', 'csv']],
  ['audio', ['mp3', 'wav', 'm4a', 'ogg', 'flac', 'aac', 'opus', 'aiff']],
  [
    'code',
    [
      'ts', 'tsx', 'js', 'jsx', 'mjs', 'cjs', 'rs', 'py', 'go', 'java', 'rb', 'c', 'h', 'cc',
      'cpp', 'hpp', 'cs', 'php', 'swift', 'kt', 'scala', 'sh', 'bash', 'zsh', 'sql', 'json',
      'yaml', 'yml', 'toml', 'html', 'css', 'scss', 'vue', 'svelte', 'lua', 'pl', 'r',
    ],
  ],
]

/** The extension of a filename, lowercased, or '' where there is none. */
export function extensionOf(filename: string): string {
  const base = filename.split(/[\\/]/).pop() ?? ''
  const dot = base.lastIndexOf('.')
  // A leading dot is a dotfile, not an extension: `.gitignore` has none.
  if (dot <= 0 || dot === base.length - 1) return ''
  return base.slice(dot + 1).toLowerCase()
}

/**
 * Which kind a dropped file records.
 *
 * Anything unrecognised is a `link`, which is the honest answer: we know a file
 * of that name exists somewhere and nothing more.
 */
export function kindForFilename(filename: string): AttachmentKind {
  const ext = extensionOf(filename)
  if (!ext) return 'link'
  for (const [kind, exts] of BY_KIND) if (exts.includes(ext)) return kind
  return 'link'
}

/** Whether some dropped text reads as a URL this can point at. */
export function isUrl(text: string): boolean {
  return /^(https?|mailto|ftp):/i.test(text.trim())
}

/**
 * A readable name for dropped text.
 *
 * For a URL that is its last meaningful segment, else its host — a name like
 * `spec.md` or `example.com` is what somebody scanning a node wants, and the
 * whole URL is kept in `ref` anyway. For prose it is the first line, trimmed.
 */
export function nameForText(text: string): string {
  const trimmed = text.trim()
  if (isUrl(trimmed)) {
    const withoutScheme = trimmed.replace(/^[a-z]+:\/\//i, '').replace(/^mailto:/i, '')
    const [hostAndPath = ''] = withoutScheme.split(/[?#]/)
    const parts = hostAndPath.split('/').filter(Boolean)
    return (parts.length > 1 ? parts[parts.length - 1] : parts[0]) || trimmed
  }
  return (trimmed.split('\n')[0] ?? '').trim() || trimmed
}

/** What a browser handed the canvas. Reduced to the two things that matter, so
 *  a test can express a drop without constructing a `DataTransfer`. */
export interface DropPayload {
  /** Dropped files, in the order the browser listed them. Names only — the bytes
   *  are deliberately never read. */
  files: readonly { name: string }[]
  /** `text/uri-list` if the browser offered one, else `text/plain`. */
  text: string
}

export interface DropGists {
  /** Says a file attachment is a reference to a file living somewhere else. */
  file: string
}

export interface DropResult {
  /** What to add, already clamped to what still fits. */
  add: AttachDraft[]
  /** How many the node had no room for. Non-zero means say so. */
  refused: number
}

/**
 * What a drop onto a node with `existing` attachments should add.
 *
 * Files win over text when a browser offers both, because a file drag also
 * carries its path as text and adding the same thing twice is never what was
 * meant.
 *
 * The cap is applied by FILLING and then reporting: several files dropped on a
 * nearly-full node add what fits and say how many did not, rather than either
 * refusing the whole gesture or silently losing the tail.
 */
export function draftsForDrop(
  payload: DropPayload,
  existing: number,
  gists: DropGists,
): DropResult {
  const room = Math.max(0, MAX_ATTACHMENTS - existing)
  const wanted: AttachDraft[] = []

  if (payload.files.length > 0) {
    for (const file of payload.files) {
      wanted.push({
        kind: kindForFilename(file.name),
        name: file.name.slice(0, MAX_ATTACHMENT_NAME),
        gist: gists.file,
        // Empty, deliberately. A browser gives a dropped file no path, and
        // inventing one would be a pointer to nowhere.
        ref: '',
      })
    }
  } else if (payload.text.trim()) {
    const text = payload.text.trim()
    wanted.push({
      kind: 'link',
      name: nameForText(text).slice(0, MAX_ATTACHMENT_NAME),
      gist: '',
      ref: text.slice(0, MAX_ATTACHMENT_REF),
    })
  }

  return { add: wanted.slice(0, room), refused: Math.max(0, wanted.length - room) }
}

// One section's prose, bound to that section's own fragment.
//
// The whole trick is one line:
//
//   Collaboration.configure({ document: ydoc, fragment })
//
// `@tiptap/extension-collaboration` resolves
// `this.options.fragment ? this.options.fragment : document.getXmlFragment(field)`,
// so an editor can be pointed at a fragment nested inside the document rather
// than at a top-level field. A section's prose lives in its NODE — an
// `XmlFragment` under `prose` in the node's `Y.Map` — so the map and the document
// are two renderings of one thing rather than two copies kept in step.
//
// Everything else here follows from that. There is no per-section socket and no
// per-section document: the page opens the MAP's session once and hands every
// section the same `ydoc` and the same provider, so five hundred sections cost
// one connection.
//
// The caret is shared the same way, and it behaves correctly for a reason worth
// knowing: `y-prosemirror` stores a cursor in awareness as a position RELATIVE
// to the type its editor is bound to, and resolving one against a different type
// yields nothing. So a remote caret is drawn in the section that person is
// actually typing in, and nowhere else, with no coordination between the mounted
// editors at all.
import { useEffect, useRef } from 'react'
import { EditorContent, useEditor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import Collaboration from '@tiptap/extension-collaboration'
import CollaborationCaret from '@tiptap/extension-collaboration-caret'
import { ySyncPluginKey } from 'y-prosemirror'
import type { WebsocketProvider } from 'y-websocket'
import type * as Y from 'yjs'

import type { Editor } from '@tiptap/react'

import { BlockId } from '@/lib/block-id'
import { HighlightBlocks, setHighlightedBlocks } from '@/lib/block-highlight'

/** How long a person stops typing before the edit counts as settled. */
const SETTLE_MS = 2500

export interface SectionEditorProps {
  ydoc: Y.Doc
  /** The node's own `prose` fragment. See `proseOf` in lib/mindmap-crdt. */
  fragment: Y.XmlFragment
  provider: WebsocketProvider
  /** The name and colour collaborators see against this caret. */
  display: string
  color: string
  canWrite: boolean
  /**
   * Called when a LOCAL edit settles — on blur, or after a pause.
   *
   * The trace is sparse by contract: an entry per keystroke buries every entry
   * worth reading, and the server refuses nothing here, so the restraint has to
   * be kept on this side. Remote changes never call it, or two people editing
   * two sections would each file entries for the other.
   */
  onSettled: () => void
  /**
   * The blocks a pending proposal is about, as a sorted space-joined string.
   *
   * A string rather than a `Set` on purpose: highlighting goes through a
   * ProseMirror transaction, so the effect that dispatches it must fire when the
   * SET changed and not merely when the page re-rendered — and a fresh `Set` is
   * a new identity every render. See `highlightKeyFor`.
   */
  highlight?: string
  /**
   * The editor, handed up as it mounts and `null` as it goes.
   *
   * Accepting a proposal means applying its ops to THIS section's document, and
   * building ProseMirror content needs the editor's exact schema — the same
   * reason the server does not do it. So the page holds the editors and the
   * decision is made against the right one.
   */
  onEditor?: (editor: Editor | null) => void
  label: string
}

export default function SectionEditor({
  ydoc,
  fragment,
  provider,
  display,
  color,
  canWrite,
  onSettled,
  highlight = '',
  onEditor,
  label,
}: SectionEditorProps) {
  const dirty = useRef(false)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  // The callback is read at fire time rather than captured, so a settle already
  // in flight is not held to whatever the page's closure looked like 2.5 seconds
  // ago — and the effect below does not have to tear the editor down to pick up
  // a new one.
  const settle = useRef(onSettled)
  settle.current = onSettled

  const editor = useEditor(
    {
      editable: canWrite,
      extensions: [
        // Collaboration owns the document, so StarterKit's own history has to
        // go: an undo stack that does not know about remote edits would undo
        // somebody else's sentence.
        StarterKit.configure({ undoRedo: false }),
        Collaboration.configure({ document: ydoc, fragment }),
        CollaborationCaret.configure({ provider, user: { name: display, color } }),
        // Only a writer mints block ids — an `appendTransaction` runs regardless
        // of `editable`, so a reader would otherwise change the shared document
        // by opening a section.
        BlockId.configure({ canWrite }),
        // A decoration, never a mark: a mark would be content, written into the
        // shared document and synced to everybody, which would break the very
        // rule the highlight illustrates. See `lib/block-highlight.ts`.
        HighlightBlocks,
      ],
      editorProps: {
        attributes: {
          class: 'prose prose-neutral dark:prose-invert max-w-none focus:outline-none px-1 py-1',
          'aria-label': label,
        },
      },
      // The app is client-only, so this is simply the correct answer; Tiptap
      // warns without it when an editor mounts during an SSR-shaped render.
      immediatelyRender: false,
    },
    [ydoc, fragment, provider, canWrite],
  )

  // The page keeps a handle on this editor while it is mounted, and loses it the
  // moment it is not — an offscreen section has no editor, and a decision made
  // against a stale one would apply ops to a document nobody is looking at.
  const register = useRef(onEditor)
  register.current = onEditor
  useEffect(() => {
    register.current?.(editor ?? null)
    return () => register.current?.(null)
  }, [editor])

  useEffect(() => {
    if (!editor) return
    setHighlightedBlocks(editor.view, new Set(highlight ? highlight.split(' ') : []))
  }, [editor, highlight])

  useEffect(() => {
    if (!editor) return
    const flush = () => {
      if (timer.current) {
        clearTimeout(timer.current)
        timer.current = null
      }
      if (!dirty.current) return
      dirty.current = false
      settle.current()
    }
    const onTransaction = ({ transaction }: { transaction: { docChanged: boolean } }) => {
      const tr = transaction as unknown as { docChanged: boolean; getMeta: (k: unknown) => unknown }
      if (!tr.docChanged) return
      // A transaction carrying y-sync's own key is the socket applying somebody
      // ELSE's edit. Filing "edited" for it would credit this reader with a
      // paragraph they only watched arrive.
      if (tr.getMeta(ySyncPluginKey)) return
      dirty.current = true
      if (timer.current) clearTimeout(timer.current)
      timer.current = setTimeout(flush, SETTLE_MS)
    }
    editor.on('transaction', onTransaction)
    editor.on('blur', flush)
    return () => {
      editor.off('transaction', onTransaction)
      editor.off('blur', flush)
      // Leaving the section is settling: an unmount with an edit still pending
      // would lose the only record that it happened.
      flush()
    }
  }, [editor])

  if (!editor) return null
  return <EditorContent editor={editor} />
}

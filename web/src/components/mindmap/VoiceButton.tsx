// Dictation, as one button on the map's action bar.
//
// A brainstorm happens at talking speed, and the 280-character node cap says a
// node is one sentence somebody said. So the map takes dictation: press once,
// talk, and every finished sentence lands as a thought.
//
// The audio never touches Takomo — the page streams it to the provider itself
// (see `lib/voice.ts`), which is what keeps a recording out of the server's log
// and off its disk.
import { useCallback, useEffect, useRef, useState } from 'react'
import { MicIcon, SquareIcon } from 'lucide-react'
import { startDictation, type VoiceHandle } from '@/lib/voice'
import { cn } from '@/lib/utils'

export interface VoiceButtonLabels {
  /** The resting button: start listening. */
  start: string
  /** While listening: press again to stop. */
  stop: string
  /** Between the press and the first audio frame. */
  starting: string
  /** Shown beside the button while a sentence is still being spoken. */
  hearing: string
  /** No microphone, or permission refused. */
  noMic: string
  /** The session ended on its own. */
  lost: string
}

export interface VoiceButtonProps {
  token: string
  /** A finished sentence. The map turns it into a node. */
  onText: (text: string) => void
  /** Called when a session starts, so the map can anchor the branch. */
  onStart?: () => void
  onError: (message: string) => void
  disabled?: boolean
  labels: VoiceButtonLabels
  className?: string
}

export function VoiceButton({
  token,
  onText,
  onStart,
  onError,
  disabled,
  labels,
  className,
}: VoiceButtonProps) {
  const [state, setState] = useState<'off' | 'starting' | 'on'>('off')
  const [partial, setPartial] = useState('')
  const handle = useRef<VoiceHandle | null>(null)
  // Held in a ref as well as state: the unmount cleanup below must release the
  // microphone even though it cannot read the state it closed over.
  const stop = useCallback(() => {
    handle.current?.stop()
    handle.current = null
    setState('off')
    setPartial('')
  }, [])

  // A page left with the microphone open is the failure worth engineering
  // against: it survives navigation and the person cannot see it.
  useEffect(() => () => handle.current?.stop(), [])

  const toggle = useCallback(async () => {
    if (state !== 'off') {
      stop()
      return
    }
    setState('starting')
    try {
      onStart?.()
      handle.current = await startDictation(token, {
        onOpen: () => setState('on'),
        onPartial: setPartial,
        onFinal: (text) => {
          setPartial('')
          onText(text)
        },
        onError: (why) => {
          onError(why === 'lost' ? labels.lost : why)
          stop()
        },
      })
    } catch (e) {
      // A refused microphone and a missing one arrive the same way, and neither
      // is a fault worth a stack trace — it is an answer.
      const denied =
        e instanceof DOMException || (e instanceof Error && /permission|device/i.test(e.message))
      onError(denied ? labels.noMic : e instanceof Error ? e.message : labels.lost)
      stop()
    }
  }, [state, stop, token, onText, onStart, onError, labels.noMic, labels.lost])

  return (
    <span className={cn('flex items-center gap-1.5', className)}>
      <button
        type="button"
        disabled={disabled}
        aria-pressed={state === 'on'}
        onClick={() => void toggle()}
        className={cn(
          'flex cursor-pointer items-center gap-1 rounded-md border px-2.5 py-1 text-[12px] font-[650] disabled:opacity-40',
          state === 'off'
            ? 'border-border text-muted-foreground hover:text-foreground'
            : 'border-destructive text-destructive',
        )}
      >
        {state === 'off' ? (
          <MicIcon className="size-3.5" aria-hidden="true" />
        ) : (
          <SquareIcon className="size-3.5" aria-hidden="true" />
        )}
        {state === 'off' ? labels.start : state === 'starting' ? labels.starting : labels.stop}
      </button>
      {/* The words so far, never committed: seeing them is how you know it is
          hearing you, and a half-sentence node is not what you asked for. */}
      {state === 'on' && (
        <span className="text-muted-foreground max-w-[22ch] truncate text-[11.5px] italic">
          {partial || labels.hearing}
        </span>
      )}
    </span>
  )
}

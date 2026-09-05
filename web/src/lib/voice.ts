// Dictation on the map: speech in, nodes out.
//
// A brainstorm arrives faster than anyone types it, and the 280-character node
// cap exists precisely because a node is one sentence somebody said. So the map
// takes dictation, and each finished sentence becomes a node.
//
// The audio goes STRAIGHT to the provider from this page. It never reaches
// Takomo, which keeps a recording out of the server's request log and off its
// disk; what the page holds is a short-lived token the server mints, never the
// account key. `POST /v1/speech/token` is the whole server side of this.
import { api } from './api'

/** Sample rate the provider expects, and what we resample the mic to. */
const SAMPLE_RATE = 16_000

export interface VoiceHandle {
  /** Stop listening and release the microphone. Safe to call twice. */
  stop: () => void
}

export interface VoiceCallbacks {
  /** A finished sentence. This is what becomes a node. */
  onFinal: (text: string) => void
  /** The words so far, for showing that it is listening. Never committed. */
  onPartial?: (text: string) => void
  /** Anything that stops the session, in words a person can act on. */
  onError: (message: string) => void
  /** Told when the socket is up, so the button can stop saying "starting". */
  onOpen?: () => void
}

/**
 * Begin a dictation session.
 *
 * Rejects rather than throwing asynchronously, so a caller gets one place to
 * handle "no microphone", "permission refused" and "dictation is off" — all of
 * which are ordinary answers rather than faults.
 */
export async function startDictation(
  token: string,
  cb: VoiceCallbacks,
): Promise<VoiceHandle> {
  const speechToken = await mintToken(token)

  // Ask for the microphone BEFORE opening the socket: a refused permission
  // should not leave a connection open, and the browser's own prompt is the
  // slowest step.
  const media = await navigator.mediaDevices.getUserMedia({
    audio: { channelCount: 1, sampleRate: SAMPLE_RATE, echoCancellation: true },
  })

  const ctx = new AudioContext({ sampleRate: SAMPLE_RATE })
  const socket = new WebSocket(
    `wss://streaming.assemblyai.com/v3/ws?sample_rate=${SAMPLE_RATE}&token=${encodeURIComponent(speechToken)}`,
  )
  socket.binaryType = 'arraybuffer'

  let stopped = false
  const stop = () => {
    if (stopped) return
    stopped = true
    try {
      if (socket.readyState === WebSocket.OPEN) socket.close()
    } catch {
      // Already gone; nothing to do.
    }
    for (const track of media.getTracks()) track.stop()
    void ctx.close().catch(() => {})
  }

  socket.onopen = () => {
    cb.onOpen?.()
    const source = ctx.createMediaStreamSource(media)
    // `ScriptProcessorNode` is deprecated and is still the only thing that works
    // without shipping a separate worklet file; the bundle here is one document
    // and an AudioWorklet needs a URL of its own. Revisit if that changes.
    const node = ctx.createScriptProcessor(4096, 1, 1)
    node.onaudioprocess = (e) => {
      if (stopped || socket.readyState !== WebSocket.OPEN) return
      const input = e.inputBuffer.getChannelData(0)
      // Float32 [-1,1] to the 16-bit PCM the provider wants.
      const pcm = new Int16Array(input.length)
      for (let i = 0; i < input.length; i++) {
        const s = Math.max(-1, Math.min(1, input[i] ?? 0))
        pcm[i] = s < 0 ? s * 0x8000 : s * 0x7fff
      }
      socket.send(pcm.buffer)
    }
    source.connect(node)
    node.connect(ctx.destination)
  }

  socket.onmessage = (e) => {
    let msg: { transcript?: string; end_of_turn?: boolean; error?: string }
    try {
      msg = JSON.parse(String(e.data))
    } catch {
      return
    }
    if (msg.error) {
      cb.onError(msg.error)
      stop()
      return
    }
    const text = (msg.transcript ?? '').trim()
    if (!text) return
    // Only a finished turn becomes a node. A partial is shown and thrown away:
    // committing one would put half a sentence on the map and then a whole one.
    if (msg.end_of_turn) cb.onFinal(text)
    else cb.onPartial?.(text)
  }

  socket.onerror = () => {
    cb.onError('lost')
    stop()
  }

  return { stop }
}

/** The server's exchange: an account key it holds for a token this page may. */
async function mintToken(token: string): Promise<string> {
  const res = (await api(token, '/speech/token', { method: 'POST' })) as {
    token?: string
  }
  if (!res.token) throw new Error('no token')
  return res.token
}

// Dictation's button, and the two failures worth pinning.
//
// A partial transcript must never become a node — the map would grow half a
// sentence and then the whole one — and the microphone must be released when the
// page goes away, because a page left listening is a failure nobody can see.
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'

import { VoiceButton, type VoiceButtonLabels } from './VoiceButton'
import * as voice from '@/lib/voice'

const LABELS: VoiceButtonLabels = {
  start: 'Dictate',
  stop: 'Stop',
  starting: 'Starting…',
  hearing: 'Listening…',
  noMic: 'No microphone.',
  lost: 'Dictation stopped.',
}

let callbacks: voice.VoiceCallbacks | null = null
const stop = vi.fn()

beforeEach(() => {
  callbacks = null
  stop.mockClear()
  vi.spyOn(voice, 'startDictation').mockImplementation(async (_token, cb) => {
    callbacks = cb
    cb.onOpen?.()
    return { stop }
  })
})

function setup(onText = vi.fn(), onError = vi.fn()) {
  const view = render(
    <VoiceButton token="tk_x" onText={onText} onError={onError} labels={LABELS} />,
  )
  return { view, onText, onError }
}

describe('VoiceButton', () => {
  it('commits a finished sentence and never a partial one', async () => {
    const { onText } = setup()
    fireEvent.click(screen.getByRole('button', { name: /Dictate/ }))
    await waitFor(() => expect(callbacks).not.toBeNull())

    act(() => callbacks!.onPartial?.('we should cache the'))
    expect(onText).not.toHaveBeenCalled()
    expect(screen.getByText('we should cache the')).toBeTruthy()

    act(() => callbacks!.onFinal('We should cache the price list.'))
    expect(onText).toHaveBeenCalledWith('We should cache the price list.')
    expect(onText).toHaveBeenCalledTimes(1)
  })

  it('releases the microphone when the page goes away', async () => {
    const { view } = setup()
    fireEvent.click(screen.getByRole('button', { name: /Dictate/ }))
    await waitFor(() => expect(callbacks).not.toBeNull())
    view.unmount()
    expect(stop).toHaveBeenCalled()
  })

  it('stops on a second press', async () => {
    setup()
    fireEvent.click(screen.getByRole('button', { name: /Dictate/ }))
    await waitFor(() => expect(screen.getByRole('button', { name: /Stop/ })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: /Stop/ }))
    expect(stop).toHaveBeenCalled()
    expect(screen.getByRole('button', { name: /Dictate/ })).toBeTruthy()
  })

  it('says what a refused microphone was, rather than throwing', async () => {
    vi.spyOn(voice, 'startDictation').mockRejectedValue(
      new DOMException('Permission denied', 'NotAllowedError'),
    )
    const { onError } = setup()
    fireEvent.click(screen.getByRole('button', { name: /Dictate/ }))
    await waitFor(() => expect(onError).toHaveBeenCalledWith(LABELS.noMic))
    expect(screen.getByRole('button', { name: /Dictate/ })).toBeTruthy()
  })

  it('is inert without write scope', () => {
    render(
      <VoiceButton
        token="tk_x"
        onText={vi.fn()}
        onError={vi.fn()}
        disabled
        labels={LABELS}
      />,
    )
    expect(screen.getByRole('button', { name: /Dictate/ })).toHaveProperty('disabled', true)
  })
})

import { AnswerLinkDialog } from '@takomo/web'

const noop = () => {}

/**
 * What an outside expert is handed: a single-use, expiring URL that answers
 * exactly one question. Shown ONCE because the server keeps no plaintext, so
 * "copy it now" is a fact rather than a nag.
 */
export function Open() {
  return (
    <AnswerLinkDialog
      link={{
        url: 'https://takomo.example.com/board#a=tka_OlkHLdQgThIFvc0EpsrPJn0Sv3P6CL0o',
        expires_at: '2026-08-15T12:45:18.691Z',
      }}
      lang="en"
      onClose={noop}
      labels={{
        title: 'Answer link created',
        body: 'A single-use link that lets someone answer just this one question — no token of their own needed. Share it only with the intended person.',
        once: 'Shown only once — copy it now.',
        copy: 'Copy',
        copied: 'Copied',
        done: 'Done',
        validUntil: 'Valid until',
        copyFail: 'Copy failed — select it manually.',
      }}
    />
  )
}

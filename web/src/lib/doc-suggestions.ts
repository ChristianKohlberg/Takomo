// What ⌘K offers, given what is selected.
//
// A pure function over the editor's state, so it is unit-testable without a
// browser — and so the one interesting decision in it stays visible: the entries
// are not generic text actions. Several come straight out of the design the
// documents surface exists to serve, because the menu is the only place a person
// ever meets that vocabulary.
//
// "Als Zusage formulieren" is the clearest example. A Zusage is a sentence you
// could check the software against, and the difference between one and an
// ordinary true sentence is the distinction the whole model rests on. A menu
// that only offered "shorten" and "rephrase" would leave a person to discover
// that on their own, which is to say never.
import type { Suggestion } from '@/pages/documents/CommandMenu'

export interface SuggestionContext {
  /** The node type the caret is in: `paragraph`, `heading`, `bulletList`, … */
  blockKind: string | null
  /** Whether the user has selected words rather than just placed a caret. */
  hasSelection: boolean
}

export interface SuggestionLabels {
  tighten: string
  simpler: string
  asCommitment: string
  asCommitmentHint: string
  findContradiction: string
  findContradictionHint: string
  asList: string
  expandSection: string
  splitSection: string
  addMissingItems: string
  whatIsMissing: string
  whatIsMissingHint: string
  contradictions: string
  summarize: string
  addHeadings: string
  addHeadingsHint: string
}

/**
 * The entries for this context, most useful first.
 *
 * Order matters more than length: the list is arrow-navigable and the first
 * entry is what Enter runs, so the thing somebody most often wants has to be at
 * the top.
 */
export function suggestionsFor(
  ctx: SuggestionContext,
  t: SuggestionLabels,
): Suggestion[] {
  if (ctx.hasSelection) {
    return [
      { label: t.tighten, instruction: 'Kürze die markierte Stelle, ohne Inhalt zu verlieren.' },
      { label: t.simpler, instruction: 'Formuliere die markierte Stelle einfacher.' },
      {
        label: t.asCommitment,
        hint: t.asCommitmentHint,
        instruction:
          'Formuliere die markierte Stelle als Zusage: ein Satz, bei dem man nachschauen kann, ' +
          'ob die Software sich wirklich so verhält. Nenne, wer handelt und was beobachtbar ' +
          'passiert. Erfinde kein Verhalten, das im Dokument nicht steht — wenn es sich so nicht ' +
          'formulieren lässt, sag das in der Zusammenfassung.',
      },
      {
        label: t.findContradiction,
        hint: t.findContradictionHint,
        instruction:
          'Prüfe, ob die markierte Stelle einer anderen Stelle im Dokument widerspricht. ' +
          'Wenn ja, markiere den Widerspruch als offene Frage direkt darunter, statt eine der ' +
          'beiden Seiten zu ändern — welche stimmt, entscheidet ein Mensch.',
      },
      { label: t.asList, instruction: 'Mach aus der markierten Stelle eine Liste.' },
    ]
  }

  if (ctx.blockKind === 'heading') {
    return [
      {
        label: t.expandSection,
        instruction:
          'Formuliere den Abschnitt unter dieser Überschrift aus. Stütze dich nur auf das, was ' +
          'im Dokument steht; wo etwas fehlt, schreib eine offene Frage statt einer Vermutung.',
      },
      { label: t.splitSection, instruction: 'Teile diesen Abschnitt in sinnvolle Unterabschnitte mit Überschriften.' },
      { label: t.tighten, instruction: 'Kürze diesen Abschnitt, ohne Inhalt zu verlieren.' },
    ]
  }

  if (ctx.blockKind === 'bulletList' || ctx.blockKind === 'orderedList') {
    return [
      {
        label: t.addMissingItems,
        instruction:
          'Ergänze fehlende Punkte in dieser Liste — aber nur solche, die sich aus dem übrigen ' +
          'Dokument ergeben. Erfinde nichts dazu.',
      },
      { label: t.tighten, instruction: 'Kürze die Punkte dieser Liste.' },
    ]
  }

  if (ctx.blockKind) {
    return [
      { label: t.tighten, instruction: 'Kürze diesen Absatz, ohne Inhalt zu verlieren.' },
      { label: t.simpler, instruction: 'Formuliere diesen Absatz einfacher.' },
      {
        label: t.asCommitment,
        hint: t.asCommitmentHint,
        instruction:
          'Formuliere diesen Absatz als Zusage: ein Satz, bei dem man nachschauen kann, ob die ' +
          'Software sich wirklich so verhält. Nenne, wer handelt und was beobachtbar passiert. ' +
          'Erfinde kein Verhalten, das im Dokument nicht steht.',
      },
      { label: t.asList, instruction: 'Mach aus diesem Absatz eine Liste.' },
    ]
  }

  // Nothing selected and no caret in a block: the document as a whole.
  return [
    {
      label: t.whatIsMissing,
      hint: t.whatIsMissingHint,
      instruction:
        'Lies das Dokument und schreib an den passenden Stellen offene Fragen dazu, wo etwas ' +
        'fehlt, das man wissen müsste, um das zu bauen. Beantworte sie nicht — eine offene ' +
        'Frage ist eine Entscheidung, die noch niemand getroffen hat.',
    },
    {
      label: t.contradictions,
      instruction:
        'Suche Stellen im Dokument, die einander widersprechen. Markiere jeden Widerspruch als ' +
        'offene Frage, statt eine der beiden Seiten zu ändern.',
    },
    {
      label: t.addHeadings,
      hint: t.addHeadingsHint,
      instruction:
        'Gliedere das Dokument mit Überschriften, ohne den Text selbst zu ändern.',
    },
    { label: t.summarize, instruction: 'Schreib eine kurze Zusammenfassung an den Anfang des Dokuments.' },
  ]
}

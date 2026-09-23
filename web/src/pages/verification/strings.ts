// DE/EN strings for the Tests view of the specification workspace.
// EN is the reference shape — a DE table that drifts is a compile error.
import { defineStrings } from '@/lib/i18n'

export const STR = defineStrings({
  en: {
    heading: 'What must work?',
    intro:
      'A behavior describes what the software must do. Link the tests that show it; CI and agents report runs, and the status follows from the latest results.',
    newBehavior: 'New behavior',
    refresh: 'Refresh',
    loading: 'Loading …',
    needWrite: "Changing behaviors needs a token with the 'write' scope.",
    noAccess: 'Cannot access tests in {project}',
    loadFailed: 'Could not load data',
    noAccessHint:
      'Choose a project you can access, or use your profile to sign in with an authorized account.',
    retryHint: 'Refresh to try again.',

    statusVerified: 'Verified',
    statusFailing: 'Failing',
    statusStale: 'Stale',
    statusUntested: 'Untested',
    freshness: 'Verified means a linked test passed in the last {days} days.',
    overview: 'Behavior overview',

    search: 'Search behaviors',
    filterSection: 'Filter section',
    allSections: 'All sections',
    noSection: 'No section',
    clearFilters: 'Clear filters',
    emptyFiltered: 'No behaviors match these filters.',
    emptySection: 'No behaviors for this section yet.',
    empty:
      'No behaviors yet. Describe what the software must do, link the tests that show it, and let CI or an agent report runs.',
    truncated: 'Showing {shown} of {total}. Narrow the list with search or a status.',

    tests: 'tests',
    test: 'test',
    noTests: 'No linked tests',
    back: 'All behaviors',

    fTitle: 'Title',
    fTitlePh: 'Failed save keeps edits and allows retry',
    fStatement: 'Behavior',
    fStatementPh: 'When saving fails, the edits stay in the editor and retrying saves them.',
    fStatementHint: 'Plain language, or given / when / then. Markdown is supported.',
    fSection: 'Section of the specification',
    fTests: 'Linked tests',
    fTestsPh: 'playwright:editor.spec.ts › keeps edits on save failure',
    fTestsHint: 'One test key per line, exactly as CI or the agent reports it.',
    create: 'Create',
    cancel: 'Cancel',
    save: 'Save',
    saved: 'Saved.',
    delete: 'Delete',
    confirmDelete:
      'Delete this behavior? Its links are removed; reported results stay with their runs.',
    titleRequired: 'A behavior needs a title.',

    linkedTests: 'Linked tests',
    addTest: 'Add test',
    addTestPh: 'Test key',
    removeTest: 'Remove {test}',
    notReported: 'not reported yet',
    history: 'History',
    noHistory: 'No results reported for the linked tests yet.',
    pass: 'pass',
    fail: 'fail',

    unlinked: 'Reported tests without a behavior',
    unlinkedHint:
      'These keys were reported in runs but no behavior links them. Link each to the behavior it shows, or add a behavior for it.',
    linkTo: 'Link to …',
    linkToLabel: 'Link {test} to a behavior',
  },
  de: {
    heading: 'Was muss funktionieren?',
    intro:
      'Ein Verhalten beschreibt, was die Software tun muss. Verknüpfe die Tests, die es zeigen; CI und Agenten melden Läufe, und der Status folgt aus den neuesten Ergebnissen.',
    newBehavior: 'Neues Verhalten',
    refresh: 'Aktualisieren',
    loading: 'Laden …',
    needWrite: "Zum Ändern von Verhalten braucht das Token den Scope 'write'.",
    noAccess: 'Kein Zugriff auf Tests in {project}',
    loadFailed: 'Daten konnten nicht geladen werden',
    noAccessHint:
      'Wähle ein zugängliches Projekt oder melde dich im Profil mit einem berechtigten Konto an.',
    retryHint: 'Bitte erneut aktualisieren.',

    statusVerified: 'Verifiziert',
    statusFailing: 'Fehlschlagend',
    statusStale: 'Veraltet',
    statusUntested: 'Ungetestet',
    freshness:
      'Verifiziert heißt: Ein verknüpfter Test war in den letzten {days} Tagen erfolgreich.',
    overview: 'Verhaltensübersicht',

    search: 'Verhalten suchen',
    filterSection: 'Abschnitt filtern',
    allSections: 'Alle Abschnitte',
    noSection: 'Kein Abschnitt',
    clearFilters: 'Filter löschen',
    emptyFiltered: 'Kein Verhalten passt zu diesen Filtern.',
    emptySection: 'Noch kein Verhalten für diesen Abschnitt.',
    empty:
      'Noch kein Verhalten. Beschreibe, was die Software tun muss, verknüpfe die Tests, die es zeigen, und lass CI oder einen Agenten Läufe melden.',
    truncated: '{shown} von {total} angezeigt. Grenze die Liste mit Suche oder Status ein.',

    tests: 'Tests',
    test: 'Test',
    noTests: 'Keine verknüpften Tests',
    back: 'Alle Verhalten',

    fTitle: 'Titel',
    fTitlePh: 'Fehlgeschlagenes Speichern behält Änderungen und erlaubt Wiederholen',
    fStatement: 'Verhalten',
    fStatementPh:
      'Wenn das Speichern fehlschlägt, bleiben die Änderungen im Editor und ein erneuter Versuch speichert sie.',
    fStatementHint: 'Klare Sprache oder Gegeben / Wenn / Dann. Markdown wird unterstützt.',
    fSection: 'Abschnitt der Spezifikation',
    fTests: 'Verknüpfte Tests',
    fTestsPh: 'playwright:editor.spec.ts › keeps edits on save failure',
    fTestsHint: 'Ein Testschlüssel pro Zeile, genau so, wie CI oder der Agent ihn meldet.',
    create: 'Anlegen',
    cancel: 'Abbrechen',
    save: 'Speichern',
    saved: 'Gespeichert.',
    delete: 'Löschen',
    confirmDelete:
      'Dieses Verhalten löschen? Die Verknüpfungen werden entfernt; gemeldete Ergebnisse bleiben bei ihren Läufen.',
    titleRequired: 'Ein Verhalten braucht einen Titel.',

    linkedTests: 'Verknüpfte Tests',
    addTest: 'Test hinzufügen',
    addTestPh: 'Testschlüssel',
    removeTest: '{test} entfernen',
    notReported: 'noch nicht gemeldet',
    history: 'Verlauf',
    noHistory: 'Für die verknüpften Tests wurden noch keine Ergebnisse gemeldet.',
    pass: 'bestanden',
    fail: 'fehlgeschlagen',

    unlinked: 'Gemeldete Tests ohne Verhalten',
    unlinkedHint:
      'Diese Schlüssel wurden in Läufen gemeldet, aber kein Verhalten verknüpft sie. Verknüpfe jeden mit dem Verhalten, das er zeigt, oder lege eines dafür an.',
    linkTo: 'Verknüpfen mit …',
    linkToLabel: '{test} mit einem Verhalten verknüpfen',
  },
})

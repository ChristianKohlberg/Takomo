// DE/EN strings for /verification.
// EN is the reference shape — a DE table that drifts is a compile error.
import { defineStrings } from '@/lib/i18n'

export const STR = defineStrings({
  en: {
    board: 'Board',
    inbox: 'Inbox',
    initiatives: 'Initiatives',
    schedules: 'Schedules',
    verification: 'Verification',
    environments: 'Environments',
    settings: 'Settings',

    gateTokenSub:
      "A token with 'read'. Filing a check needs 'write'; approving a case needs 'human'.",
    gateLabel: 'API token',
    gateOpen: 'Open',
    tokenNeeded: 'A token is needed to read this project.',
    gateNoRead: "That token cannot read. Use one carrying the 'read' scope.",
    requestFailed: 'The request failed.',
    allProjects: 'All projects',
    project: 'Project',
    projectSearch: 'Search projects',
    projectNoMatch: 'No project matches',
    navExpand: 'Expand',
    navCollapse: 'Collapse',
    navAccount: 'Account',
    signOut: 'Sign out',
    refresh: 'Refresh',

    newCheck: 'New check',
    needWrite: "Filing a check needs a token with the 'write' scope.",
    needHuman:
      "Approving a case asserts that a PERSON checked it, so it needs a token with the 'human' scope. An agent records what it observed.",

    empty:
      'No checks yet. A check is ONE action with ONE entry precondition at ONE layer — a create form, a finalize step and a cancel action are three checks, not one.',
    emptyHint: 'File the ones an initiative already agreed on, and the gap stops being invisible.',

    gateClear: 'Nothing blocking',
    gateBlocked: 'Blocked',
    gateBlockedSub: 'blocking case(s) outstanding',
    gateAdvisory: 'advisory outstanding — these do not block',
    worklistAgent: 'for an agent',
    worklistHuman: 'for a person',
    worklistCases: 'case(s)',
    minutes: 'min',

    unassigned: 'No initiative',
    unassignedHint: 'Checks nobody filed under an initiative.',
    lastVerified: 'last verified',
    neverVerified: 'never verified',

    stateFailed: 'failed',
    stateStale: 'stale',
    stateNever: 'never run',
    stateUnreachable: 'unreachable',
    stateVerified: 'verified',
    stateApproved: 'approved',
    stateNone: 'no cases',

    reasonStale: 'stale',
    reasonExpired: 'expired',
    reasonFailed: 'failed',
    reasonNever: 'never run',
    reasonAwaitingHuman: 'awaiting you',

    orphanGlobs: 'claims paths that matched no file in the newest release',
    showCases: 'Cases',
    hideCases: 'Hide',
    noCases:
      'No cases filed. A check with no cases verifies nothing — file the parameter assignments it crosses.',
    approve: 'Approve',
    markPass: 'Pass',
    markFail: 'Fail',
    failNeedsNote: 'Say what went wrong — a failure nobody described is one nobody can act on.',
    notePlaceholder: 'What went wrong?',
    recorded: 'Recorded.',
    archiveCheck: 'Archive',
    confirmArchiveCheck:
      'Archive this check? Its cases and their verdict history are kept — a check no longer worth running is still evidence of what was once verified.',

    copyBrief: 'Copy agent brief',
    briefCopied: 'Brief copied',

    fTitle: 'What one action does this verify?',
    fTitlePh: 'Split an invoice across two entities',
    fInitiative: 'Initiative that agreed it',
    fInitiativeNone: '— none —',
    fLayer: 'Layer',
    fLayerHint:
      'A rule enforced only in the interface passes at the API layer, so the two verdicts are not interchangeable. One check covers ONE layer.',
    fSeverity: 'Severity',
    fSeverityHint: 'Only blocking severity blocks a release gate.',
    fPrecondition: 'Entry precondition',
    fPreconditionPh: 'A finalised invoice with at least two billable entities on it.',
    fBody: 'The traversal to follow',
    fBodyPh: 'Open the invoice, split it, confirm each share reaches AP.',
    fGlobs: 'Paths it exercises',
    fGlobsPh: 'src/billing/split/**',
    fGlobsHint:
      'One per line. Hand-declared and known to rot — a release push reports the ones that match nothing.',
    create: 'File check',
    cancel: 'Cancel',
  },
  de: {
    board: 'Board',
    inbox: 'Inbox',
    initiatives: 'Initiativen',
    schedules: 'Zeitpläne',
    verification: 'Verifizierung',
    environments: 'Umgebungen',
    settings: 'Einstellungen',

    gateTokenSub:
      "Ein Token mit 'read'. Zum Anlegen einer Prüfung wird 'write' benötigt, zum Freigeben eines Falls 'human'.",
    gateLabel: 'API-Token',
    gateOpen: 'Öffnen',
    tokenNeeded: 'Zum Lesen dieses Projekts wird ein Token benötigt.',
    gateNoRead: "Dieses Token kann nicht lesen. Bitte eines mit 'read' verwenden.",
    requestFailed: 'Die Anfrage ist fehlgeschlagen.',
    allProjects: 'Alle Projekte',
    project: 'Projekt',
    projectSearch: 'Projekte suchen',
    projectNoMatch: 'Kein Projekt passt',
    navExpand: 'Ausklappen',
    navCollapse: 'Einklappen',
    navAccount: 'Konto',
    signOut: 'Abmelden',
    refresh: 'Aktualisieren',

    newCheck: 'Neue Prüfung',
    needWrite: "Zum Anlegen einer Prüfung wird ein Token mit 'write' benötigt.",
    needHuman:
      "Eine Freigabe bestätigt, dass ein MENSCH geprüft hat, und benötigt daher ein Token mit 'human'. Ein Agent hält fest, was er beobachtet hat.",

    empty:
      'Noch keine Prüfungen. Eine Prüfung ist EINE Aktion mit EINER Eingangsbedingung auf EINER Ebene — ein Anlegen-Formular, ein Abschluss-Schritt und ein Storno sind drei Prüfungen, nicht eine.',
    emptyHint:
      'Legen Sie die an, auf die sich eine Initiative bereits geeinigt hat — dann ist die Lücke nicht mehr unsichtbar.',

    gateClear: 'Nichts blockiert',
    gateBlocked: 'Blockiert',
    gateBlockedSub: 'blockierende Fälle offen',
    gateAdvisory: 'empfehlend offen — diese blockieren nicht',
    worklistAgent: 'für einen Agenten',
    worklistHuman: 'für einen Menschen',
    worklistCases: 'Fälle',
    minutes: 'Min',

    unassigned: 'Keine Initiative',
    unassignedHint: 'Prüfungen, die niemand einer Initiative zugeordnet hat.',
    lastVerified: 'zuletzt verifiziert',
    neverVerified: 'nie verifiziert',

    stateFailed: 'fehlgeschlagen',
    stateStale: 'veraltet',
    stateNever: 'nie gelaufen',
    stateUnreachable: 'nicht erreichbar',
    stateVerified: 'verifiziert',
    stateApproved: 'freigegeben',
    stateNone: 'keine Fälle',

    reasonStale: 'veraltet',
    reasonExpired: 'abgelaufen',
    reasonFailed: 'fehlgeschlagen',
    reasonNever: 'nie gelaufen',
    reasonAwaitingHuman: 'wartet auf Sie',

    orphanGlobs: 'beansprucht Pfade, die im neuesten Release auf keine Datei passten',
    showCases: 'Fälle',
    hideCases: 'Ausblenden',
    noCases:
      'Keine Fälle hinterlegt. Eine Prüfung ohne Fälle verifiziert nichts — hinterlegen Sie die Parameterbelegungen.',
    approve: 'Freigeben',
    markPass: 'Bestanden',
    markFail: 'Fehlgeschlagen',
    failNeedsNote:
      'Bitte angeben, was schiefging — ein Fehlschlag, den niemand beschreibt, ist einer, mit dem niemand etwas anfangen kann.',
    notePlaceholder: 'Was ging schief?',
    recorded: 'Festgehalten.',
    archiveCheck: 'Archivieren',
    confirmArchiveCheck:
      'Diese Prüfung archivieren? Ihre Fälle und deren Urteilsverlauf bleiben erhalten — eine Prüfung, die sich nicht mehr lohnt, belegt weiterhin, was einmal verifiziert war.',

    copyBrief: 'Auftrag kopieren',
    briefCopied: 'Auftrag kopiert',

    fTitle: 'Welche eine Aktion wird hier verifiziert?',
    fTitlePh: 'Eine Rechnung auf zwei Einheiten aufteilen',
    fInitiative: 'Initiative, die sie vereinbart hat',
    fInitiativeNone: '— keine —',
    fLayer: 'Ebene',
    fLayerHint:
      'Eine Regel, die nur in der Oberfläche greift, passiert die API-Ebene — die Urteile sind daher nicht austauschbar. Eine Prüfung deckt EINE Ebene ab.',
    fSeverity: 'Schweregrad',
    fSeverityHint: 'Nur „blocking“ blockiert ein Release-Gate.',
    fPrecondition: 'Eingangsbedingung',
    fPreconditionPh: 'Eine abgeschlossene Rechnung mit mindestens zwei abrechenbaren Einheiten.',
    fBody: 'Der abzulaufende Weg',
    fBodyPh: 'Rechnung öffnen, aufteilen, prüfen, dass jeder Anteil in der Kreditorenbuchhaltung ankommt.',
    fGlobs: 'Pfade, die sie ausübt',
    fGlobsPh: 'src/billing/split/**',
    fGlobsHint:
      'Einer pro Zeile. Von Hand gepflegt und veraltet zwangsläufig — ein Release meldet die, die auf nichts mehr passen.',
    create: 'Prüfung anlegen',
    cancel: 'Abbrechen',
  },
})

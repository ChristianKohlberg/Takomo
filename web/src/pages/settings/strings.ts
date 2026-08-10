// DE/EN strings for /settings.
// EN is the reference shape — a DE table that drifts is a compile error.
import { defineStrings } from '@/lib/i18n'

export const STR = defineStrings({
  en: {
    settings: 'Settings',
    board: 'Board',
    inbox: 'Inbox',
    initiatives: 'Initiatives',
    schedules: 'Schedules',

    gateTokenSub: "An admin token. Without 'admin' this page has nothing to show you.",
    gateLabel: 'API token',
    gateOpen: 'Open',
    gateEmpty: 'Paste a token to continue.',

    // Section rail
    navOverview: 'Overview',
    navOverviewHint: 'The token you are signed in with',
    navData: 'Data',
    navDataHint: 'Back up the whole store',
    navAccess: 'Access',
    navAccessHint: 'Tokens that can reach the API',
    navProjects: 'Projects',
    navProjectsHint: 'Create and remove projects',

    signOut: 'Sign out',
    notAdminTitle: 'This token is not an admin',
    notAdmin:
      "Everything on this page needs the 'admin' scope. Sign in with an admin token, or mint one on the server:",
    notAdminCmd: 'takomo token create --actor you --scopes read,write,human,admin',

    // Overview
    overviewTitle: 'This token',
    overviewSub: 'Who the API thinks you are, and what it will let you do.',
    factActor: 'Actor',
    factScopes: 'Scopes',
    factProjects: 'Projects',
    factTokenId: 'Token id',
    allProjects: 'All projects',

    // Data
    dataTitle: 'Database export',
    dataSub:
      'Downloads the entire store as one SQLite file — every project, plus tokens, OAuth clients, shares, answer grants and the event log.',
    dataHow:
      'Taken with VACUUM INTO, so it is a consistent snapshot including writes still in the WAL. Open it directly with sqlite3.',
    dataWarnTitle: 'Treat the file as a credential',
    dataWarn:
      'It contains token hashes and OAuth client secrets. Anyone holding it holds the contents of this Takomo.',
    dataScopedTitle: 'Not available to this token',
    dataScoped:
      'A whole-database export cannot be narrowed to a project allowlist, so a token limited to specific projects is refused. Use an unrestricted admin token, or export a single project from the board.',
    dataBtn: 'Download database',
    dataBusy: 'Preparing…',
    dataDone: 'Downloaded {name} ({size})',

    // Access
    accessTitle: 'Tokens',
    accessSub: 'Every credential that can reach this API. Revoking one takes effect immediately.',
    accessEmpty: 'No tokens yet.',
    accessNew: 'New token',
    tokThisToken: 'this token',
    tokScopes: 'Scopes',
    tokProjects: 'Projects',
    tokLastUsed: 'last used',
    tokNeverUsed: 'never used',
    tokRevoked: 'revoked',
    tokExpired: 'expired',
    tokRevoke: 'Revoke',

    newTokTitle: 'New token',
    newTokSub: 'The plaintext is shown once, at creation, and never again.',
    newTokActor: 'Actor',
    newTokActorPh: 'agent:ci',
    newTokActorHint: 'Who this credential belongs to — a person, a machine, a job.',
    newTokScopes: 'Scopes',
    newTokRead: 'Read tickets, questions and events.',
    newTokWrite: 'Create and change tickets; claim work.',
    newTokHuman: 'Answer questions and approve schedules.',
    newTokAdmin: 'Manage tokens and projects, and export the database.',
    newTokProjects: 'Limit to projects',
    newTokProjectsHint: 'This token will only see the projects selected above.',
    newTokAll: 'Nothing selected — the token reaches every project.',
    newTokCreate: 'Create token',
    newTokCancel: 'Cancel',
    newTokNeedActor: 'An actor is required.',
    newTokNeedScope: 'Pick at least one scope.',

    revealTitle: 'Copy this token now',
    revealSub:
      'It is shown once and cannot be recovered — Takomo stores only a hash. If you lose it, revoke it and mint another.',
    revealCopy: 'Copy',
    revealCopied: 'Copied',
    revealDone: 'Done',

    confirmRevokeTitle: 'Revoke this token?',
    confirmRevokeBody: 'Anything using {actor} stops working immediately. This cannot be undone.',
    confirmRevokeYes: 'Revoke token',

    // Projects
    projTitle: 'Projects',
    projSub: 'Each project has its own workflow, tickets and schedules.',
    projEmpty: 'No projects yet.',
    projNew: 'New project',
    projDelete: 'Delete',
    newProjTitle: 'New project',
    newProjSub: 'The id prefixes every ticket in this project and cannot be changed later.',
    newProjId: 'Id',
    newProjIdPh: 'demo',
    newProjIdHint: 'Lowercase letters, digits and dashes.',
    newProjIdInvalid: 'Use lowercase letters, digits and dashes, starting with a letter or digit.',
    newProjName: 'Name',
    newProjNamePh: 'Demo',
    newProjNameHint: 'Shown in the project picker. Defaults to the id.',
    newProjCreate: 'Create project',
    newProjCancel: 'Cancel',
    confirmDeleteProjTitle: 'Delete this project?',
    confirmDeleteProjBody:
      'Deleting {id} removes its tickets, questions, schedules and checklist data. This cannot be undone.',
    confirmDeleteProjYes: 'Delete project',

    cancel: 'Cancel',
    requestFailed: 'Request failed.',
  },
  de: {
    settings: 'Einstellungen',
    board: 'Board',
    inbox: 'Posteingang',
    initiatives: 'Initiativen',
    schedules: 'Zeitpläne',

    gateTokenSub: "Ein Admin-Token. Ohne 'admin' hat diese Seite nichts zu zeigen.",
    gateLabel: 'API-Token',
    gateOpen: 'Öffnen',
    gateEmpty: 'Zum Fortfahren ein Token einfügen.',

    navOverview: 'Übersicht',
    navOverviewHint: 'Das Token, mit dem du angemeldet bist',
    navData: 'Daten',
    navDataHint: 'Den gesamten Speicher sichern',
    navAccess: 'Zugriff',
    navAccessHint: 'Tokens, die die API erreichen',
    navProjects: 'Projekte',
    navProjectsHint: 'Projekte anlegen und entfernen',

    signOut: 'Abmelden',
    notAdminTitle: 'Dieses Token ist kein Admin',
    notAdmin:
      "Alles auf dieser Seite benötigt den 'admin'-Scope. Melde dich mit einem Admin-Token an oder erzeuge eines auf dem Server:",
    notAdminCmd: 'takomo token create --actor du --scopes read,write,human,admin',

    overviewTitle: 'Dieses Token',
    overviewSub: 'Wofür die API dich hält und was sie dich tun lässt.',
    factActor: 'Akteur',
    factScopes: 'Scopes',
    factProjects: 'Projekte',
    factTokenId: 'Token-Id',
    allProjects: 'Alle Projekte',

    dataTitle: 'Datenbank-Export',
    dataSub:
      'Lädt den gesamten Speicher als eine SQLite-Datei herunter — alle Projekte, dazu Tokens, OAuth-Clients, Freigaben, Antwort-Grants und das Ereignisprotokoll.',
    dataHow:
      'Per VACUUM INTO erstellt, also ein konsistenter Snapshot inklusive Schreibvorgängen, die noch im WAL liegen. Direkt mit sqlite3 zu öffnen.',
    dataWarnTitle: 'Behandle die Datei wie ein Geheimnis',
    dataWarn:
      'Sie enthält Token-Hashes und OAuth-Client-Secrets. Wer sie hat, hat den Inhalt dieses Takomo.',
    dataScopedTitle: 'Für dieses Token nicht verfügbar',
    dataScoped:
      'Ein Gesamt-Export lässt sich nicht auf eine Projekt-Freigabeliste einschränken, daher wird ein auf bestimmte Projekte begrenztes Token abgelehnt. Nutze ein uneingeschränktes Admin-Token oder exportiere ein einzelnes Projekt vom Board aus.',
    dataBtn: 'Datenbank herunterladen',
    dataBusy: 'Wird vorbereitet…',
    dataDone: '{name} heruntergeladen ({size})',

    accessTitle: 'Tokens',
    accessSub:
      'Alle Zugangsdaten, die diese API erreichen können. Ein Widerruf wirkt sofort.',
    accessEmpty: 'Noch keine Tokens.',
    accessNew: 'Neues Token',
    tokThisToken: 'dieses Token',
    tokScopes: 'Scopes',
    tokProjects: 'Projekte',
    tokLastUsed: 'zuletzt benutzt',
    tokNeverUsed: 'nie benutzt',
    tokRevoked: 'widerrufen',
    tokExpired: 'abgelaufen',
    tokRevoke: 'Widerrufen',

    newTokTitle: 'Neues Token',
    newTokSub: 'Der Klartext wird einmal bei der Erstellung angezeigt und nie wieder.',
    newTokActor: 'Akteur',
    newTokActorPh: 'agent:ci',
    newTokActorHint: 'Wem dieses Token gehört — einer Person, einer Maschine, einem Job.',
    newTokScopes: 'Scopes',
    newTokRead: 'Tickets, Fragen und Ereignisse lesen.',
    newTokWrite: 'Tickets anlegen und ändern; Arbeit übernehmen.',
    newTokHuman: 'Fragen beantworten und Zeitpläne freigeben.',
    newTokAdmin: 'Tokens und Projekte verwalten und die Datenbank exportieren.',
    newTokProjects: 'Auf Projekte begrenzen',
    newTokProjectsHint: 'Dieses Token sieht nur die oben ausgewählten Projekte.',
    newTokAll: 'Nichts ausgewählt — das Token erreicht alle Projekte.',
    newTokCreate: 'Token erstellen',
    newTokCancel: 'Abbrechen',
    newTokNeedActor: 'Ein Akteur ist erforderlich.',
    newTokNeedScope: 'Mindestens einen Scope auswählen.',

    revealTitle: 'Kopiere dieses Token jetzt',
    revealSub:
      'Es wird einmal angezeigt und ist nicht wiederherstellbar — Takomo speichert nur einen Hash. Wenn du es verlierst, widerrufe es und erzeuge ein neues.',
    revealCopy: 'Kopieren',
    revealCopied: 'Kopiert',
    revealDone: 'Fertig',

    confirmRevokeTitle: 'Dieses Token widerrufen?',
    confirmRevokeBody:
      'Alles, was {actor} nutzt, funktioniert sofort nicht mehr. Das lässt sich nicht rückgängig machen.',
    confirmRevokeYes: 'Token widerrufen',

    projTitle: 'Projekte',
    projSub: 'Jedes Projekt hat einen eigenen Workflow, eigene Tickets und Zeitpläne.',
    projEmpty: 'Noch keine Projekte.',
    projNew: 'Neues Projekt',
    projDelete: 'Löschen',
    newProjTitle: 'Neues Projekt',
    newProjSub:
      'Die Id steht vor jedem Ticket dieses Projekts und kann später nicht geändert werden.',
    newProjId: 'Id',
    newProjIdPh: 'demo',
    newProjIdHint: 'Kleinbuchstaben, Ziffern und Bindestriche.',
    newProjIdInvalid:
      'Kleinbuchstaben, Ziffern und Bindestriche verwenden, beginnend mit Buchstabe oder Ziffer.',
    newProjName: 'Name',
    newProjNamePh: 'Demo',
    newProjNameHint: 'Wird in der Projektauswahl angezeigt. Standard ist die Id.',
    newProjCreate: 'Projekt erstellen',
    newProjCancel: 'Abbrechen',
    confirmDeleteProjTitle: 'Dieses Projekt löschen?',
    confirmDeleteProjBody:
      'Mit {id} werden dessen Tickets, Fragen, Zeitpläne und Checklisten-Daten entfernt. Das lässt sich nicht rückgängig machen.',
    confirmDeleteProjYes: 'Projekt löschen',

    cancel: 'Abbrechen',
    requestFailed: 'Anfrage fehlgeschlagen.',
  },
})

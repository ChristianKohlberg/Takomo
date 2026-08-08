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

    // Session
    sessionTitle: 'This token',
    sessionActor: 'actor',
    sessionScopes: 'scopes',
    sessionProjects: 'projects',
    allProjects: 'all projects',
    signOut: 'Sign out',
    notAdmin:
      "This token does not carry the 'admin' scope, so the sections below are unavailable. Sign in with an admin token, or mint one on the server with: takomo token create --actor you --scopes read,write,human,admin",

    // Export
    exportTitle: 'Database export',
    exportBody:
      'Downloads the entire store as one SQLite file — every project, plus tokens, OAuth clients, shares, answer grants and the event log. Taken with VACUUM INTO, so it is a consistent snapshot you can open directly in sqlite3.',
    exportWarn:
      'The file contains token hashes and OAuth client secrets. Treat it like a credential, not like a report.',
    exportScoped:
      'This token is limited to specific projects, so it cannot take a whole-database export. Use an unrestricted admin token, or export one project from the board.',
    exportBtn: 'Download database',
    exportBusy: 'Preparing…',
    exportDone: 'Downloaded {name} ({size})',

    // Tokens
    tokensTitle: 'Tokens',
    tokensSub: 'Every credential that can reach the API. Revoking one takes effect immediately.',
    tokensEmpty: 'No tokens yet.',
    tokensNew: 'New token',
    tokenActor: 'Actor',
    tokenScopes: 'Scopes',
    tokenProjects: 'Projects (blank = all)',
    tokenCreate: 'Create',
    tokenCancel: 'Cancel',
    tokenRevoke: 'Revoke',
    tokenRevoked: 'revoked',
    tokenExpired: 'expired',
    tokenNeverUsed: 'never used',
    tokenLastUsed: 'last used',
    tokenCreatedOnce:
      'Copy this now — it is shown once and never again. Takomo stores only a hash.',
    tokenCopy: 'Copy',
    tokenCopied: 'Copied',
    tokenDone: 'Done',
    confirmRevoke: 'Revoke this token? Anything using it stops working at once.',

    // Projects
    projectsTitle: 'Projects',
    projectsSub: 'Deleting a project removes its tickets, questions and schedules.',
    projectsEmpty: 'No projects yet.',
    projectsNew: 'New project',
    projectId: 'Id',
    projectName: 'Name',
    projectWorkflow: 'Workflow',
    projectCreate: 'Create',
    projectDelete: 'Delete',
    confirmDeleteProject: 'Delete project "{id}" and everything in it? This cannot be undone.',

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

    sessionTitle: 'Dieses Token',
    sessionActor: 'Akteur',
    sessionScopes: 'Scopes',
    sessionProjects: 'Projekte',
    allProjects: 'alle Projekte',
    signOut: 'Abmelden',
    notAdmin:
      "Dieses Token hat keinen 'admin'-Scope, daher sind die folgenden Bereiche nicht verfügbar. Melde dich mit einem Admin-Token an oder erzeuge eines auf dem Server: takomo token create --actor du --scopes read,write,human,admin",

    exportTitle: 'Datenbank-Export',
    exportBody:
      'Lädt den gesamten Speicher als eine SQLite-Datei herunter — alle Projekte, dazu Tokens, OAuth-Clients, Freigaben, Antwort-Grants und das Ereignisprotokoll. Per VACUUM INTO erstellt, also ein konsistenter Snapshot, den du direkt in sqlite3 öffnen kannst.',
    exportWarn:
      'Die Datei enthält Token-Hashes und OAuth-Client-Secrets. Behandle sie wie ein Geheimnis, nicht wie einen Bericht.',
    exportScoped:
      'Dieses Token ist auf bestimmte Projekte beschränkt und kann daher keinen Gesamt-Export erstellen. Nutze ein uneingeschränktes Admin-Token oder exportiere ein einzelnes Projekt vom Board aus.',
    exportBtn: 'Datenbank herunterladen',
    exportBusy: 'Wird vorbereitet…',
    exportDone: '{name} heruntergeladen ({size})',

    tokensTitle: 'Tokens',
    tokensSub:
      'Alle Zugangsdaten, die die API erreichen können. Ein Widerruf wirkt sofort.',
    tokensEmpty: 'Noch keine Tokens.',
    tokensNew: 'Neues Token',
    tokenActor: 'Akteur',
    tokenScopes: 'Scopes',
    tokenProjects: 'Projekte (leer = alle)',
    tokenCreate: 'Erstellen',
    tokenCancel: 'Abbrechen',
    tokenRevoke: 'Widerrufen',
    tokenRevoked: 'widerrufen',
    tokenExpired: 'abgelaufen',
    tokenNeverUsed: 'nie benutzt',
    tokenLastUsed: 'zuletzt benutzt',
    tokenCreatedOnce:
      'Jetzt kopieren — es wird nur einmal angezeigt. Takomo speichert nur einen Hash.',
    tokenCopy: 'Kopieren',
    tokenCopied: 'Kopiert',
    tokenDone: 'Fertig',
    confirmRevoke: 'Dieses Token widerrufen? Alles, was es nutzt, funktioniert sofort nicht mehr.',

    projectsTitle: 'Projekte',
    projectsSub: 'Ein gelöschtes Projekt nimmt seine Tickets, Fragen und Zeitpläne mit.',
    projectsEmpty: 'Noch keine Projekte.',
    projectsNew: 'Neues Projekt',
    projectId: 'Id',
    projectName: 'Name',
    projectWorkflow: 'Workflow',
    projectCreate: 'Erstellen',
    projectDelete: 'Löschen',
    confirmDeleteProject:
      'Projekt "{id}" und alles darin löschen? Das lässt sich nicht rückgängig machen.',

    requestFailed: 'Anfrage fehlgeschlagen.',
  },
})

export interface SyncSession {
 object: string; kind: string; session: string; token: string; can_write: boolean
 display: string; expires_at: string; url: string; room: string
}
export function syncBase(session: { url: string }): string {
 if (/^wss?:/.test(session.url)) return session.url
 return `${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}${session.url}`
}

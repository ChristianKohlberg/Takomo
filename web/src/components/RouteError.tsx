import { detectLocale } from '@/lib/i18n'

/** Works even when a lazy route failed to download after a deployment. */
export function RouteError() {
  const de = detectLocale() === 'de'
  return <main className="bg-background text-foreground flex min-h-dvh items-center justify-center p-6">
    <section role="alert" className="w-full max-w-lg rounded-lg border border-border bg-card p-6 shadow-sm">
      <p className="text-muted-foreground mb-2 text-sm font-semibold">Takomo</p>
      <h1 className="mb-3 text-xl font-semibold">{de ? 'Diese Seite konnte nicht geladen werden.' : 'This page could not be loaded.'}</h1>
      <p className="text-muted-foreground mb-6 text-sm">{de
        ? 'Möglicherweise wurde Takomo gerade aktualisiert. Lade die Seite neu, um es erneut zu versuchen.'
        : 'Takomo may have just been updated. Reload the page to try again.'}</p>
      <div className="flex flex-wrap gap-3">
        <button type="button" className="bg-primary text-primary-foreground rounded-md px-4 py-2 text-sm font-medium" onClick={() => window.location.reload()}>{de ? 'Seite neu laden' : 'Reload page'}</button>
        <a href="/board" className="rounded-md border border-border px-4 py-2 text-sm font-medium">{de ? 'Zum Board' : 'Go to board'}</a>
      </div>
    </section>
  </main>
}

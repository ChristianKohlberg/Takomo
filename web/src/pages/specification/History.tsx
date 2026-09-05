import { useCallback, useEffect, useRef, useState } from 'react'
import { useLocation, useNavigate } from 'react-router'
import { Download, History as HistoryIcon } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from '@/components/ui/sheet'
import { useProjectUpdates } from '@/hooks/useProjectUpdates'
import {
  checkpoint,
  compareVersions,
  historyPage,
  mergeHistoryPage,
  savedVersion,
  type VersionDetail,
  type VersionPage,
} from '@/lib/spec-history'
import { useSpecification } from './context'

const words = {
  en: {
    title: 'Version history',
    intro:
      'Revisit earlier wording and agreements. Your shared specification stays open underneath.',
    all: 'All saves',
    named: 'Checkpoints',
    noCheckpoints:
      'No named checkpoints yet. Choose All saves to review earlier versions.',
    empty: 'No saved versions yet. History begins with the next saved edit.',
    baseline: 'History starts here',
    saved: 'Saved version',
    checkpoint: 'Name the latest saved version',
    reviewLatest: 'Review latest version',
    newer: 'Review the latest saved version before naming an agreement.',
    name: 'Checkpoint name',
    create: 'Save checkpoint',
    waiting: 'Wait until your changes are saved to create a checkpoint.',
    refresh: 'Refresh',
    more: 'Older versions',
    preview: 'Read version',
    compare: 'Compare with',
    none: 'No comparison',
    added: 'Added',
    removed: 'Removed',
    changed: 'Changed',
    unchanged: 'No section changes.',
    before: 'Before',
    after: 'After',
    download: 'Download version',
    loading: 'Loading version…',
    select: 'Choose a version to read or compare.',
    count: 'saved versions',
    rels: 'Relationships changed',
    details: 'Other changes',
    formatting: 'Text formatting changed.',
    readonly: 'Read-only · earlier saved content',
    current: 'Latest saved',
    error: 'Could not load history. Try Refresh.',
    recovery:
      'Downloads keep formatting and map details. Restoring a version into the live document is not available yet.',
  },
  de: {
    title: 'Versionsverlauf',
    intro:
      'Frühere Formulierungen und Vereinbarungen nachlesen. Die gemeinsame Spezifikation bleibt darunter geöffnet.',
    all: 'Alle Speicherstände',
    named: 'Meilensteine',
    noCheckpoints:
      'Noch keine benannten Meilensteine. Unter Alle Speicherstände findest du frühere Versionen.',
    empty:
      'Noch keine gespeicherten Versionen. Der Verlauf beginnt mit der nächsten gespeicherten Änderung.',
    baseline: 'Hier beginnt der Verlauf',
    saved: 'Gespeicherte Version',
    checkpoint: 'Letzten Speicherstand benennen',
    reviewLatest: 'Letzte Version ansehen',
    newer:
      'Sieh dir den letzten Speicherstand an, bevor du die Vereinbarung benennst.',
    name: 'Name des Meilensteins',
    create: 'Meilenstein speichern',
    waiting: 'Warte, bis deine Änderungen gespeichert sind.',
    refresh: 'Aktualisieren',
    more: 'Ältere Versionen',
    preview: 'Version lesen',
    compare: 'Vergleichen mit',
    none: 'Kein Vergleich',
    added: 'Hinzugefügt',
    removed: 'Entfernt',
    changed: 'Geändert',
    unchanged: 'Keine Abschnittsänderungen.',
    before: 'Vorher',
    after: 'Nachher',
    download: 'Version herunterladen',
    loading: 'Version laden…',
    select: 'Wähle eine Version zum Lesen oder Vergleichen.',
    count: 'gespeicherte Versionen',
    rels: 'Beziehungen geändert',
    details: 'Weitere Änderungen',
    formatting: 'Textformatierung geändert.',
    readonly: 'Nur lesen · früherer Speicherstand',
    current: 'Zuletzt gespeichert',
    error: 'Verlauf konnte nicht geladen werden. Bitte aktualisieren.',
    recovery:
      'Downloads bewahren Formatierung und Map-Details. Das Wiederherstellen einer Version im Live-Dokument ist noch nicht verfügbar.',
  },
}
export default function History() {
  const { token, lang, project, map, scopes, saveState, onError } =
    useSpecification()!
  const location = useLocation(),
    navigate = useNavigate()
  const query = new URLSearchParams(location.search)
  const open = query.get('history') === '1'
  const w = words[lang]
  const mapId = map?.id
  const [automaticVersion, setAutomaticVersion] = useState(0)
  const [named, setNamed] = useState(false)
  const [page, setPage] = useState<VersionPage | null>(null)
  const [detail, setDetail] = useState<VersionDetail | null>(null)
  const [comparison, setComparison] = useState<VersionDetail | null>(null)
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)
  const [loading, setLoading] = useState(false)
  const epoch = useRef(0)
  const selected = Number(query.get('version')) || automaticVersion
  const against = Number(query.get('compare')) || 0
  const change = (values: Record<string, string | null>) => {
    const next = new URLSearchParams(location.search)
    for (const [key, value] of Object.entries(values)) {
      if (value === null) next.delete(key)
      else next.set(key, value)
    }
    navigate({ search: next.toString() })
  }
  const refresh = useCallback(async () => {
    if (!mapId || !open) return
    const request = ++epoch.current
    try {
      const result = await historyPage(token, mapId, null, named)
      if (request === epoch.current) {
        setPage((current) => mergeHistoryPage(current, result))
        setAutomaticVersion(
          (current) => current || result.items[0]?.version || 0,
        )
        setFailed(false)
      }
    } catch (error) {
      if (request === epoch.current) {
        setFailed(true)
        onError(error)
      }
    }
  }, [token, mapId, open, named, onError])
  useEffect(() => {
    const ref = epoch
    setPage(null)
    setAutomaticVersion(0)
    void refresh()
    return () => {
      ref.current++
    }
  }, [refresh])
  // Reuse the workspace's project socket. No extra polling or collaboration room.
  useProjectUpdates(token, project, refresh)
  useEffect(() => {
    if (saveState === 'saved') void refresh()
  }, [saveState, refresh])
  useEffect(() => {
    setDetail(null)
    setComparison(null)
    if (!open || !mapId || !selected) return
    let cancelled = false
    setLoading(true)
    void Promise.all([
      savedVersion(token, mapId, selected),
      against ? savedVersion(token, mapId, against) : Promise.resolve(null),
    ])
      .then(([value, other]) => {
        if (!cancelled) {
          setDetail(value)
          setComparison(other)
        }
      })
      .catch((error) => {
        if (!cancelled) onError(error)
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [open, mapId, token, selected, against, onError])
  if (!map) return null
  const download = async () => {
    if (!detail) return
    const response = await fetch(
      `/v1/mindmaps/${encodeURIComponent(map.id)}/versions/${detail.version}/state`,
      { headers: { Authorization: `Bearer ${token}` } },
    )
    if (!response.ok) throw new Error(await response.text())
    const url = URL.createObjectURL(await response.blob())
    const link = document.createElement('a')
    link.href = url
    link.download = `${map.id}-v${detail.version}.yjs`
    link.click()
    setTimeout(() => URL.revokeObjectURL(url), 1000)
  }
  const fieldNames: Record<string, string> =
    lang === 'de'
      ? {
          parent: 'Übergeordneter Abschnitt',
          position: 'Reihenfolge',
          edge_label: 'Verbindungstext',
          kind: 'Art',
          color: 'Farbe',
          shape: 'Form',
          icons: 'Symbole',
          attachments: 'Anhänge',
          at: 'Position auf der Map',
          reviewed: 'Gelesen',
          origin: 'Herkunft',
          promoted: 'Verknüpfte Arbeit',
        }
      : {
          parent: 'Parent section',
          position: 'Order',
          edge_label: 'Connection label',
          kind: 'Kind',
          color: 'Color',
          shape: 'Shape',
          icons: 'Icons',
          attachments: 'Attachments',
          at: 'Map position',
          reviewed: 'Read status',
          origin: 'Origin',
          promoted: 'Linked work',
        }
  const describe = (
    field: string,
    value: unknown,
    version: VersionDetail | null,
  ) => {
    if (field === 'parent')
      return version?.nodes.find((node) => node.id === value)?.title ?? '—'
    if (value === null || value === undefined || value === '') return '—'
    return typeof value === 'object' ? JSON.stringify(value) : String(value)
  }
  const reviewingLatest =
    page?.head === 0 || (detail !== null && detail.version === page?.head)
  const canCheckpoint =
    page !== null && reviewingLatest && !loading && saveState === 'saved'
  const changes =
    detail && comparison ? compareVersions(comparison, detail) : null
  return (
    <>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => change({ history: '1', panel: null, check: null })}
      >
        <HistoryIcon className="size-4" />
        {w.title}
      </Button>
      <Sheet
        open={open}
        onOpenChange={(value) => {
          if (!value) change({ history: null, version: null, compare: null })
        }}
      >
        <SheetContent className="data-[side=right]:w-full data-[side=right]:sm:max-w-6xl gap-0 p-0">
          <SheetHeader className="border-b pr-12">
            <SheetTitle>{w.title}</SheetTitle>
            <SheetDescription>{w.intro}</SheetDescription>
          </SheetHeader>
          <div className="grid min-h-0 flex-1 grid-cols-1 overflow-y-auto md:grid-cols-[260px_minmax(0,1fr)] md:overflow-hidden">
            <aside className="min-w-0 space-y-3 border-b p-4 md:overflow-y-auto md:border-r md:border-b-0">
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  variant={named ? 'outline' : 'secondary'}
                  onClick={() => setNamed(false)}
                >
                  {w.all}
                </Button>
                <Button
                  size="sm"
                  variant={named ? 'secondary' : 'outline'}
                  onClick={() => setNamed(true)}
                >
                  {w.named}
                </Button>
              </div>
              <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                <span>
                  {page?.total ?? '…'} {w.count}
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => void refresh()}
                >
                  {w.refresh}
                </Button>
              </div>
              {failed && <p role="alert">{w.error}</p>}
              {page?.items.length === 0 && (
                <p className="text-sm text-muted-foreground">
                  {named ? w.noCheckpoints : w.empty}
                </p>
              )}
              <ol className="max-h-48 space-y-1 overflow-y-auto md:max-h-none md:overflow-visible">
                {page?.items.map((item) => (
                  <li key={item.version}>
                    <button
                      type="button"
                      aria-pressed={selected === item.version}
                      className={`w-full cursor-pointer rounded-md border p-3 text-left ${selected === item.version ? 'border-primary bg-accent' : 'border-transparent hover:bg-muted'}`}
                      onClick={() => change({ version: String(item.version) })}
                    >
                      <div className="flex justify-between gap-2 font-medium">
                        <span>v{item.version}</span>
                        {item.version === page.head && (
                          <span className="text-xs text-muted-foreground">
                            {w.current}
                          </span>
                        )}
                      </div>
                      <p className="break-words text-sm">
                        {item.checkpoints.map((c) => c.name).join(' · ') ||
                          (item.kind === 'baseline' ? w.baseline : w.saved)}
                      </p>
                      <time
                        className="text-xs text-muted-foreground"
                        dateTime={item.recorded_at}
                      >
                        {new Date(item.recorded_at).toLocaleString(lang)}
                      </time>
                    </button>
                  </li>
                ))}
              </ol>
              {page?.next_cursor !== null &&
                page?.next_cursor !== undefined && (
                  <Button
                    variant="outline"
                    disabled={busy}
                    onClick={() => {
                      const request = epoch.current
                      setBusy(true)
                      void historyPage(token, map.id, page.next_cursor, named)
                        .then((next) => {
                          if (request === epoch.current)
                            setPage(
                              (current) =>
                                current && {
                                  ...next,
                                  items: [...current.items, ...next.items],
                                },
                            )
                        })
                        .catch(onError)
                        .finally(() => setBusy(false))
                    }}
                  >
                    {w.more}
                  </Button>
                )}
            </aside>
            <main className="min-w-0 space-y-6 p-4 md:overflow-y-auto md:p-6">
              {scopes.includes('write') && (
                <form
                  className="space-y-2 rounded-lg border bg-muted/30 p-4"
                  onSubmit={(event) => {
                    event.preventDefault()
                    if (!page || busy || !canCheckpoint) return
                    setBusy(true)
                    void checkpoint(token, map.id, page.head, name)
                      .then(async (value) => {
                        setName('')
                        setDetail((current) =>
                          current?.version === value.version
                            ? { ...current, ...value }
                            : current,
                        )
                        change({ version: String(value.version) })
                        await refresh()
                      })
                      .catch(onError)
                      .finally(() => setBusy(false))
                  }}
                >
                  <label
                    className="text-sm font-medium"
                    htmlFor="checkpoint-name"
                  >
                    {w.checkpoint}
                  </label>
                  <div className="flex flex-col gap-2 md:flex-row">
                    <Input
                      id="checkpoint-name"
                      placeholder={w.name}
                      value={name}
                      maxLength={200}
                      onChange={(event) => setName(event.target.value)}
                    />
                    <Button
                      type="submit"
                      disabled={!name.trim() || busy || !canCheckpoint}
                    >
                      {w.create}
                    </Button>
                  </div>
                  {page && page.head > 0 && !reviewingLatest && (
                    <div className="space-y-2">
                      <p className="text-xs text-muted-foreground">{w.newer}</p>
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        onClick={() => change({ version: String(page.head) })}
                      >
                        {w.reviewLatest} · v{page.head}
                      </Button>
                    </div>
                  )}
                  {saveState !== 'saved' && (
                    <p className="text-xs text-muted-foreground">{w.waiting}</p>
                  )}
                </form>
              )}
              {loading ? (
                <p role="status">{w.loading}</p>
              ) : !detail ? (
                <p>{w.select}</p>
              ) : (
                <>
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div className="min-w-0 flex-1">
                      <h2 className="break-words text-lg font-semibold">
                        v{detail.version} ·{' '}
                        {detail.checkpoints.map((c) => c.name).join(' · ') ||
                          w.saved}
                      </h2>
                      <p className="text-xs text-muted-foreground">
                        {w.readonly}
                      </p>
                    </div>
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => void download().catch(onError)}
                    >
                      <Download className="size-4" />
                      {w.download}
                    </Button>
                  </div>
                  <label className="flex flex-wrap items-center gap-2 text-sm">
                    {w.compare}
                    <select
                      className="min-w-0 max-w-full rounded-md border bg-background p-2"
                      value={against}
                      onChange={(event) =>
                        change({
                          compare:
                            event.target.value === '0'
                              ? null
                              : event.target.value,
                        })
                      }
                    >
                      <option value="0">{w.none}</option>
                      {against &&
                      !page?.items.some((v) => v.version === against) ? (
                        <option value={against}>v{against}</option>
                      ) : null}
                      {page?.items
                        .filter((item) => item.version !== selected)
                        .map((item) => (
                          <option key={item.version} value={item.version}>
                            v{item.version}{' '}
                            {item.checkpoints.map((c) => c.name).join(' · ')}
                          </option>
                        ))}
                    </select>
                  </label>
                  {changes ? (
                    <div className="space-y-4">
                      {changes.length === 0 && <p>{w.unchanged}</p>}
                      {changes.map((item) => (
                        <article
                          key={item.id}
                          className="overflow-hidden rounded-lg border"
                        >
                          <header className="border-b bg-muted/40 p-3 font-medium">
                            {w[item.kind as 'added' | 'removed' | 'changed']} ·{' '}
                            {item.after?.title || item.before?.title}
                          </header>
                          <div className="grid grid-cols-1 divide-y md:grid-cols-2 md:divide-x md:divide-y-0">
                            {[item.before, item.after].map((node, index) => (
                              <div
                                key={index}
                                className="min-w-0 space-y-2 p-4"
                              >
                                <p className="text-xs text-muted-foreground">
                                  {index ? w.after : w.before} · v
                                  {index ? detail.version : comparison!.version}
                                </p>
                                <h3 className="break-words font-medium">
                                  {node?.title || '—'}
                                </h3>
                                <p className="whitespace-pre-wrap break-words">
                                  {node?.notes || '—'}
                                </p>
                              </div>
                            ))}
                          </div>
                          {item.changed.some(
                            (field) =>
                              !['title', 'notes', 'prose_xml'].includes(field),
                          ) && (
                            <details className="border-t p-3 text-xs">
                              <summary className="cursor-pointer">
                                {w.details}
                              </summary>
                              <dl className="mt-2 space-y-2">
                                {item.changed
                                  .filter(
                                    (field) =>
                                      !['title', 'notes', 'prose_xml'].includes(
                                        field,
                                      ),
                                  )
                                  .map((field) => (
                                    <div key={field} className="min-w-0">
                                      <dt className="font-medium">
                                        {fieldNames[field] ?? field}
                                      </dt>
                                      <dd className="break-words">
                                        {describe(
                                          field,
                                          item.before?.[field],
                                          comparison,
                                        )}{' '}
                                        →{' '}
                                        {describe(
                                          field,
                                          item.after?.[field],
                                          detail,
                                        )}
                                      </dd>
                                    </div>
                                  ))}
                              </dl>
                            </details>
                          )}
                          {item.changed.includes('prose_xml') &&
                            !item.changed.includes('notes') && (
                              <p className="border-t p-3 text-xs text-muted-foreground">
                                {w.formatting}
                              </p>
                            )}
                        </article>
                      ))}
                      {JSON.stringify(comparison!.relationships) !==
                        JSON.stringify(detail.relationships) && (
                        <details className="rounded-lg border p-3">
                          <summary>{w.rels}</summary>
                          <pre className="overflow-auto whitespace-pre-wrap break-all text-xs">
                            {JSON.stringify(
                              {
                                before: comparison!.relationships,
                                after: detail.relationships,
                              },
                              null,
                              2,
                            )}
                          </pre>
                        </details>
                      )}
                    </div>
                  ) : (
                    <div className="space-y-4">
                      {detail.nodes.map((node) => (
                        <article
                          key={node.id}
                          className="space-y-2 rounded-lg border p-4"
                        >
                          <p className="text-xs text-muted-foreground">
                            {node.parent
                              ? detail.nodes.find((n) => n.id === node.parent)
                                  ?.title
                              : map.title}
                          </p>
                          <h3 className="break-words text-base font-medium">
                            {node.title || '—'}
                          </h3>
                          <p className="whitespace-pre-wrap break-words">
                            {node.notes}
                          </p>
                        </article>
                      ))}
                    </div>
                  )}
                  <p className="text-xs text-muted-foreground">{w.recovery}</p>
                </>
              )}
            </main>
          </div>
        </SheetContent>
      </Sheet>
    </>
  )
}

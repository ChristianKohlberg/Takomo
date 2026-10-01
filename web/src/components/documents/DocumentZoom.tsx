import { useCallback, useEffect, useRef, useState, type CSSProperties, type RefObject } from 'react'
import { ChevronDown, Minus, MoveHorizontal, Plus } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuRadioGroup,
  DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuShortcut, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import type { Locale } from '@/lib/i18n'
import {
  correctZoomedResizeDrag, DEFAULT_ZOOM, fitZoom, formatZoom, MAX_ZOOM, MIN_ZOOM, parseZoom,
  readZoom, stepZoom, writeZoom, ZOOM_PRESETS, zoomShortcut, type ZoomSetting,
} from '@/lib/document-zoom'
import '@/styles/document-zoom.css'

export interface DocumentZoom {
  /** What the reader chose: a preset, or `fit`. */
  setting: ZoomSetting
  /** The factor in effect (`fit` resolved against the column's width). */
  value: number
  set: (setting: ZoomSetting) => void
  step: (direction: 1 | -1) => void
  reset: () => void
  /** For `.document-page` only — never for the chrome around it. */
  style: CSSProperties
}

/** Where the page's own column content starts and ends, without its padding. */
function contentWidth(column: HTMLElement): number {
  const style = getComputedStyle(column)
  return column.clientWidth - (parseFloat(style.paddingLeft) || 0) - (parseFloat(style.paddingRight) || 0)
}

/**
 * The reader's zoom for one project's document: per browser, never shared.
 * Stored beside the heading-number preferences (`takomo:document-numbering:*`)
 * as `takomo:document-zoom:<project>`; the default (100 %) is stored as nothing.
 * Also owns the keyboard shortcuts and the table-resize correction, both of
 * which exist only while the document column does.
 */
export function useDocumentZoom(project: string, columnRef: RefObject<HTMLElement | null>): DocumentZoom {
  const [saved, setSaved] = useState(() => ({ project, setting: readZoom(project) }))
  let setting = saved.setting
  if (saved.project !== project) {
    setting = readZoom(project)
    setSaved({ project, setting })
  }
  const [available, setAvailable] = useState(0)
  const fit = setting === 'fit'
  useEffect(() => {
    const column = columnRef.current
    if (!fit || !column) return
    const measure = () => setAvailable(contentWidth(column))
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(column)
    return () => observer.disconnect()
  }, [fit, columnRef])
  const value = setting === 'fit' ? fitZoom(available) : setting

  const set = useCallback((next: ZoomSetting) => {
    setSaved({ project, setting: next })
    writeZoom(project, next)
  }, [project])
  const valueRef = useRef(value)
  valueRef.current = value
  const step = useCallback((direction: 1 | -1) => set(stepZoom(valueRef.current, direction)), [set])
  const reset = useCallback(() => set(DEFAULT_ZOOM), [set])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return
      const command = zoomShortcut(event)
      if (!command) return
      event.preventDefault()
      if (command === 'reset') reset()
      else step(command === 'in' ? 1 : -1)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [step, reset])

  useEffect(() => {
    const column = columnRef.current
    if (!column) return
    return correctZoomedResizeDrag(column, () => valueRef.current)
  }, [columnRef])

  return { setting, value, set, step, reset, style: { '--document-zoom': String(value) } as CSSProperties }
}

function words(locale: Locale) {
  const de = locale === 'de'
  return {
    zoom: 'Zoom',
    zoomIn: de ? 'Vergrößern' : 'Zoom in',
    zoomOut: de ? 'Verkleinern' : 'Zoom out',
    fit: de ? 'Seitenbreite' : 'Fit width',
    reset: de ? 'Zurücksetzen' : 'Reset',
    presets: de ? 'Zoomstufe' : 'Zoom level',
  }
}

const isMac = () => typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent)
const shortcut = (key: string) => (isMac() ? `⌥⌘${key}` : `Ctrl+Alt+${key}`)

/**
 * The toolbar's zoom: a compact dropdown showing the value in effect, with the
 * −/+ steps beside it when the pane is wide (`styles/document-zoom.css`).
 */
export function DocumentZoomControl({ zoom, locale }: { zoom: DocumentZoom; locale: Locale }) {
  const t = words(locale)
  const current = formatZoom(zoom.value, locale)
  const label = zoom.setting === 'fit' ? `${t.fit} (${current})` : current
  // Announced on change (a shortcut changes it with the menu closed), not on mount.
  const [announcement, setAnnouncement] = useState('')
  const first = useRef(true)
  useEffect(() => {
    if (first.current) { first.current = false; return }
    setAnnouncement(`${t.zoom} ${label}`)
  }, [label, t.zoom])
  return <div className="flex items-center" role="group" aria-label={t.zoom}>
    <Button variant="ghost" size="icon-sm" className="document-zoom-step" aria-label={t.zoomOut} title={`${t.zoomOut} (${shortcut('−')})`}
      disabled={zoom.value <= MIN_ZOOM} onMouseDown={event => event.preventDefault()} onClick={() => zoom.step(-1)}>
      <Minus className="size-3.5" aria-hidden="true" />
    </Button>
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger aria-label={`${t.zoom}: ${label}`} title={t.zoom}
        className="text-muted-foreground hover:text-foreground hover:bg-accent focus-visible:ring-ring inline-flex min-h-8 min-w-[4.75rem] items-center justify-center gap-1 rounded-md px-1.5 py-1 text-xs tabular-nums outline-none focus-visible:ring-2">
        {/* Compact and steady: a wider label here wraps the toolbar, and a
            toolbar that wraps mid-drag moves the text under the pointer. */}
        {zoom.setting === 'fit' && <MoveHorizontal aria-hidden="true" className="size-3" />}{current}<ChevronDown aria-hidden="true" className="size-3" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-52" aria-label={t.zoom}>
        <DropdownMenuRadioGroup aria-label={t.presets} value={String(zoom.setting)} onValueChange={next => zoom.set(parseZoom(next))}>
          {ZOOM_PRESETS.map(preset => <DropdownMenuRadioItem key={preset} value={String(preset)} className="tabular-nums">
            {formatZoom(preset, locale)}
          </DropdownMenuRadioItem>)}
          <DropdownMenuRadioItem value="fit">{t.fit}</DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => zoom.step(1)} disabled={zoom.value >= MAX_ZOOM}>{t.zoomIn}<DropdownMenuShortcut>{shortcut('+')}</DropdownMenuShortcut></DropdownMenuItem>
        <DropdownMenuItem onSelect={() => zoom.step(-1)} disabled={zoom.value <= MIN_ZOOM}>{t.zoomOut}<DropdownMenuShortcut>{shortcut('−')}</DropdownMenuShortcut></DropdownMenuItem>
        <DropdownMenuItem onSelect={zoom.reset} disabled={zoom.setting === DEFAULT_ZOOM}>{t.reset}<DropdownMenuShortcut>{shortcut('0')}</DropdownMenuShortcut></DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
    <Button variant="ghost" size="icon-sm" className="document-zoom-step" aria-label={t.zoomIn} title={`${t.zoomIn} (${shortcut('+')})`}
      disabled={zoom.value >= MAX_ZOOM} onMouseDown={event => event.preventDefault()} onClick={() => zoom.step(1)}>
      <Plus className="size-3.5" aria-hidden="true" />
    </Button>
    <span role="status" className="sr-only">{announcement}</span>
  </div>
}

/** The −/+ steps inside the Tools overflow of a narrow pane. */
export function DocumentZoomSteps({ zoom, locale }: { zoom: DocumentZoom; locale: Locale }) {
  const t = words(locale)
  return <div className="document-zoom-overflow items-center gap-1" role="group" aria-label={t.zoom} data-tools-stay>
    <Button variant="ghost" size="icon-sm" aria-label={t.zoomOut} disabled={zoom.value <= MIN_ZOOM} onMouseDown={event => event.preventDefault()} onClick={() => zoom.step(-1)}>
      <Minus className="size-3.5" aria-hidden="true" />
    </Button>
    <span className="text-muted-foreground text-xs tabular-nums" aria-hidden="true">{formatZoom(zoom.value, locale)}</span>
    <Button variant="ghost" size="icon-sm" aria-label={t.zoomIn} disabled={zoom.value >= MAX_ZOOM} onMouseDown={event => event.preventDefault()} onClick={() => zoom.step(1)}>
      <Plus className="size-3.5" aria-hidden="true" />
    </Button>
  </div>
}

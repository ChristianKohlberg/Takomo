// Long document tables start collapsed to their first ten body rows.
//
// This is VIEW state only, like the collapsible block's open/closed state: it is
// never written to the Yjs document, never becomes a node attribute and never
// enters undo history. Every reader decides for themselves, and a freshly
// mounted table starts collapsed. See docs/documents.md "Tables".
//
// This file is the pure part — where to cut and what to say — shared by the
// live editor's table NodeView (`long-table-view.ts`) and the read-only
// `SavedProse` renderer, so the two can never disagree about a cut.
import { defineStrings, type Locale } from './i18n'

/** Body rows shown while a long table is collapsed. */
export const LONG_TABLE_VISIBLE_ROWS = 10

/** A row as the cut needs it: whether it is a header row, and each cell's rowspan. */
export interface LongTableRow { header: boolean; rowspans: number[] }

export interface LongTableCut {
  /** Leading rows made only of header cells; they never count and are never hidden. */
  headerRows: number
  /** Rows below the header rows. */
  bodyRows: number
  /** Rows shown while collapsed, header rows included — hide rows `visibleRows..`. */
  visibleRows: number
  /** Body rows shown while collapsed (≥ 10; more when a rowspan crosses row 10). */
  visibleBodyRows: number
}

/**
 * Where a table with more than ten body rows is cut, or `null` when it is shown whole.
 *
 * The cut lands on the first row boundary at or after the tenth body row that
 * no rowspan crosses, so a merged cell is never sliced in half. When a rowspan
 * reaches the last row from before any such boundary there is nothing to hide
 * and the table stays fully visible.
 */
export function longTableCut(rows: readonly LongTableRow[]): LongTableCut | null {
  let headerRows = 0
  while (headerRows < rows.length && rows[headerRows]!.header) headerRows++
  const bodyRows = rows.length - headerRows
  if (bodyRows <= LONG_TABLE_VISIBLE_ROWS) return null
  // reach[i] = the last row any cell starting at or before row i still covers.
  let reach = -1
  const crossed: boolean[] = []
  rows.forEach((row, index) => {
    for (const span of row.rowspans) reach = Math.max(reach, index + Math.max(1, span) - 1)
    // The boundary BEFORE row index+1 is crossed when something reaches past row `index`.
    crossed[index + 1] = reach > index
  })
  for (let boundary = headerRows + LONG_TABLE_VISIBLE_ROWS; boundary < rows.length; boundary++) {
    if (!crossed[boundary]) return { headerRows, bodyRows, visibleRows: boundary, visibleBodyRows: boundary - headerRows }
  }
  return null
}

export const LONG_TABLE_STR = defineStrings({
  en: {
    showAll: 'Show all {n} rows',
    showLess: 'Show less',
    status: 'Showing {visible} of {total} rows',
  },
  de: {
    showAll: 'Alle {n} Zeilen anzeigen',
    showLess: 'Weniger anzeigen',
    status: '{visible} von {total} Zeilen angezeigt',
  },
})

export interface LongTableLabels {
  showAll: (rows: number) => string
  showLess: string
  status: (visible: number, total: number) => string
}

export function longTableLabels(locale: Locale): LongTableLabels {
  const t = LONG_TABLE_STR[locale]
  return {
    showAll: rows => t.showAll.replace('{n}', String(rows)),
    showLess: t.showLess,
    status: (visible, total) => t.status.replace('{visible}', String(visible)).replace('{total}', String(total)),
  }
}

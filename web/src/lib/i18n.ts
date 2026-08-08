// DE/EN string tables.
//
// The pages enforced key parity with a Rust integration test
// (`spa_string_tables_agree_on_every_key` in tests/api.rs) that parsed the HTML
// and compared key sets — necessary because nothing else could see inside those
// files, and it enumerated the four pages BY HAND, so a new page went unchecked
// until someone remembered to add it.
//
// `defineStrings` replaces that with a type: EN is the reference shape, and a
// DE table missing a key or carrying an extra one is a COMPILE error. No
// enumeration to maintain, no page that can be forgotten. The runtime test
// alongside this file covers the rest (empty values, tables assembled
// dynamically), which types cannot see.
export type Locale = 'de' | 'en'

export const LOCALES: readonly Locale[] = ['de', 'en'] as const

export type Table<T extends Record<string, string>> = Record<Locale, T>

/**
 * Declare a page's strings. `en` defines the key set; `de` must match it
 * exactly — `Record<keyof T, string>` makes a missing key an error at the call
 * site and an extra key an excess-property error.
 */
export function defineStrings<T extends Record<string, string>>(tables: {
  en: T
  de: Record<keyof T, string>
}): Table<T> {
  return { en: tables.en, de: tables.de as T }
}

/** The reader's locale: an explicit choice, else the browser's, else EN. */
export function detectLocale(stored?: string | null): Locale {
  if (stored === 'de' || stored === 'en') return stored
  const nav = typeof navigator !== 'undefined' ? navigator.language : ''
  return nav && nav.toLowerCase().startsWith('de') ? 'de' : 'en'
}

/** Pick the active table. */
export function pick<T extends Record<string, string>>(table: Table<T>, locale: Locale): T {
  return table[locale]
}

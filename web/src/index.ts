// The package's public entry.
//
// It exists so the library has a conventional shape: `package.json` points
// `module`/`types` here, which is what any consumer reads — including the
// design-system converter, whose component discovery resolves the type entry
// from `types`/`typings` and finds nothing without it.
export * from './components'

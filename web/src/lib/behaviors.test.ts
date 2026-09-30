import { describe, expect, it } from 'vitest'
import { describeTest, gistOf, manualKey } from './behaviors'

describe('describeTest', () => {
  it('takes the last title of a Playwright key and keeps the file as location', () => {
    expect(
      describeTest('playwright:tests/ui-audit/login.ui.ts › Audit Anmeldung — guard › a locked-out account gets the lockout message'),
    ).toEqual({ name: 'a locked-out account gets the lockout message', kind: 'browser', location: 'login.ui.ts' })
  })

  it('reads an xUnit method with spaces and tells integration and contract tests apart', () => {
    expect(describeTest('xunit:Audit.Tests.Domain.AuditUserAccountTests.Neun_Fehlversuche_sperren_nicht')).toEqual({
      name: 'Neun Fehlversuche sperren nicht',
      kind: 'unit',
      location: 'AuditUserAccountTests',
    })
    expect(describeTest('xunit:Audit.IntegrationTests.Authentication.BruteForceSperreTests.Zehn').kind).toBe('integration')
    expect(describeTest('xunit:Fid.Api.Tests.ZusatzfeldRiskManContractTests.Picker').kind).toBe('contract')
  })

  it('reads Rust paths and agent checks, and shows anything else as it is', () => {
    expect(describeTest('cargo:api::save_conflict')).toEqual({ name: 'save conflict', kind: 'unit', location: 'api' })
    expect(describeTest('agent:failed-save-retry')).toEqual({ name: 'failed save retry', kind: 'agent', location: null })
    expect(describeTest('just a key')).toEqual({ name: 'just a key', kind: 'other', location: null })
  })
})

describe('gistOf', () => {
  it('takes the first sentence as plain text', () => {
    expect(gistOf('When **saving** fails, the edits stay. Retrying saves them.')).toBe('When saving fails, the edits stay.')
    expect(gistOf('- see [the spec](https://x.test) first\nthen more')).toBe('see the spec first then more')
    expect(gistOf('')).toBe('')
  })
  it('shortens a long first sentence', () => {
    const gist = gistOf('word '.repeat(100), 40)
    expect(gist.length).toBeLessThanOrEqual(40)
    expect(gist.endsWith('…')).toBe(true)
  })
})

describe('manual checks', () => {
  it('have one key per promise and read as a check by hand', () => {
    expect(manualKey('bhv-1')).toBe('manual:bhv-1')
    expect(describeTest(manualKey('bhv-1'))).toEqual({ name: '', kind: 'manual', location: null })
  })
})

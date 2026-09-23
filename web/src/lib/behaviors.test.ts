import { describe, expect, it } from 'vitest'
import { describeTest } from './behaviors'

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

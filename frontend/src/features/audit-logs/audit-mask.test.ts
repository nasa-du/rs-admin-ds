import { describe, expect, it } from 'vitest'
import {
  MASKED_VALUE,
  maskSensitiveDetails,
  maskedDetailsJson,
  SENSITIVE_DETAIL_KEYS,
} from './audit-mask'

describe('maskSensitiveDetails', () => {
  it('masks every forbidden key at any depth', () => {
    const masked = maskSensitiveDetails({
      username: 'alice',
      password: 'hunter2',
      nested: {
        Authorization: 'Bearer abc',
        keep: 1,
      },
    }) as Record<string, unknown>

    expect(masked).toEqual({
      username: 'alice',
      password: MASKED_VALUE,
      nested: { Authorization: MASKED_VALUE, keep: 1 },
    })
  })

  it('masks inside arrays without changing their length', () => {
    expect(
      maskSensitiveDetails([{ refresh_token: 'secret', count: 1 }, 'plain'])
    ).toEqual([{ refresh_token: MASKED_VALUE, count: 1 }, 'plain'])
  })

  it('does not mask diagnostic keys that merely contain a forbidden word', () => {
    const details = {
      permission_keys: ['audit.read'],
      key_type: 'jwt',
      old_kid: 'old',
      new_kid: 'new',
      secretive: 'keep',
    }

    expect(maskSensitiveDetails(details)).toEqual(details)
  })

  it('uses exact, case-insensitive key matching', () => {
    expect(
      maskSensitiveDetails({
        PASSWORD: 'x',
        'set-cookie': 'keep',
        cookie: 'hide me',
      })
    ).toEqual({
      PASSWORD: MASKED_VALUE,
      'set-cookie': 'keep',
      cookie: MASKED_VALUE,
    })
  })

  it('publishes the exact key catalog from the spec', () => {
    expect(SENSITIVE_DETAIL_KEYS).toEqual([
      'password',
      'password_hash',
      'temporary_password',
      'access_token',
      'refresh_token',
      'authorization',
      'cookie',
      'secret',
      'api_key',
      'private_key',
      'client_secret',
    ])
  })
})

describe('maskedDetailsJson', () => {
  it('pretty-prints masked data and never exposes the original value', () => {
    const json = maskedDetailsJson({ access_token: 'leak-me', state: 'ok' })

    expect(json).toContain('***')
    expect(json).toContain('"state": "ok"')
    expect(json).not.toContain('leak-me')
  })
})

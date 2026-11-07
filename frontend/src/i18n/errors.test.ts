import { describe, expect, it } from 'vitest'
import { ApiError } from '@/lib/api-error'
import {
  ERROR_KEY_MAP,
  type ErrorTranslator,
  FIELD_CODE_KEYS,
  PASSWORD_REASON_KEYS,
  pointerToFieldName,
  translateApiError,
  translateApiErrorFields,
  translateErrorCode,
  translateFieldError,
} from './errors'
import i18n from './index'

const t = i18n.getFixedT('en-US') as unknown as ErrorTranslator
const tZh = i18n.getFixedT('zh-CN') as unknown as ErrorTranslator

describe('error resource coverage', () => {
  it('has every mapped key in both locales', () => {
    const keys = [
      ...Object.values(ERROR_KEY_MAP),
      ...Object.values(FIELD_CODE_KEYS),
      ...Object.values(PASSWORD_REASON_KEYS),
      'errors:common.unexpected',
      'errors:common.network_error',
      'errors:common.rate_limited',
    ]

    for (const key of keys) {
      expect(i18n.exists(key, { lng: 'zh-CN' }), `zh-CN ${key}`).toBe(true)
      expect(i18n.exists(key, { lng: 'en-US' }), `en-US ${key}`).toBe(true)
    }
  })
})

describe('error code translation', () => {
  it('maps auth codes', () => {
    expect(translateErrorCode(t, 'invalid_credentials')).toBe(
      'The username or password is incorrect.'
    )
    expect(translateErrorCode(t, 'temporary_password_expired')).toBe(
      'Your temporary password has expired. Contact an administrator.'
    )
  })

  it('maps role, user, audit, and common codes', () => {
    expect(translateErrorCode(t, 'role_immutable')).toBe(
      'Built-in roles cannot be modified.'
    )
    expect(translateErrorCode(t, 'last_super_admin')).toBe(
      'At least one active super administrator must remain.'
    )
    expect(translateErrorCode(t, 'time_range_too_large')).toBe(
      'The time range is too large. Narrow it and try again.'
    )
    expect(translateErrorCode(t, 'internal_error')).toBe(
      'An internal server error occurred. Please try again later.'
    )
  })

  it('uses the auth copy for rate limiting only in an auth context', () => {
    expect(translateErrorCode(t, 'rate_limited')).toBe(
      'Too many requests. Please try again later.'
    )
    expect(translateErrorCode(t, 'rate_limited', { authContext: true })).toBe(
      'Too many sign-in attempts. Please try again later.'
    )
  })

  it('maps password policy reasons', () => {
    expect(
      translateErrorCode(t, 'password_policy_violation', {
        params: { reason: 'too_short' },
      })
    ).toBe('The password is too short.')

    expect(
      translateErrorCode(tZh, 'password_policy_violation', {
        params: { reason: 'equals_identifier' },
      })
    ).toBe('密码不能与登录标识相同。')
  })

  it('falls back to the unexpected message for unknown codes', () => {
    expect(translateErrorCode(t, 'not_in_the_catalog')).toBe(
      'An unexpected error occurred. Please try again later.'
    )
    expect(translateErrorCode(t, undefined)).toBe(
      'An unexpected error occurred. Please try again later.'
    )
  })

  it('translates raw errors and ProblemDetails through translateApiError', () => {
    expect(translateApiError(t, new TypeError('fetch failed'))).toBe(
      'The network request failed. Check your connection and try again.'
    )
    expect(translateApiError(t, { code: 'account_disabled' })).toBe(
      'This account is disabled. Contact an administrator.'
    )
    expect(translateApiError(t, new Error('boom'))).toBe(
      'An unexpected error occurred. Please try again later.'
    )
  })
})

describe('field error translation', () => {
  it('maps known pointers and validation codes', () => {
    expect(
      translateFieldError(t, {
        pointer: '/current_password',
        code: 'invalid_format',
      })
    ).toEqual({
      field: 'currentPassword',
      message: 'The format is invalid.',
    })
  })

  it('maps nested array pointers', () => {
    expect(pointerToFieldName('/urls/0/value')).toBe('urls.0.value')
  })

  it('returns null for unknown pointers so callers show a form-level error', () => {
    expect(pointerToFieldName('/unknown_field')).toBeNull()
    expect(pointerToFieldName('')).toBeNull()
  })

  it('interpolates validation params', () => {
    expect(
      translateFieldError(t, {
        pointer: '/new_password',
        code: 'min_length',
        params: { min: 15 },
      })
    ).toEqual({
      field: 'newPassword',
      message: 'Must be at least 15 characters.',
    })
  })

  it('maps password policy reasons on field errors', () => {
    expect(
      translateFieldError(t, {
        pointer: '/new_password',
        code: 'password_policy_violation',
        params: { reason: 'blocklisted' },
      })
    ).toEqual({
      field: 'newPassword',
      message: 'This password is too common. Choose another one.',
    })
  })

  it('falls back for unknown field codes', () => {
    expect(
      translateFieldError(t, {
        pointer: '/email',
        code: 'future_validation_code',
      })
    ).toEqual({ field: 'email', message: 'The value is invalid.' })
  })

  it('translates every field error on an ApiError', () => {
    const error = new ApiError({
      code: 'validation_failed',
      status: 422,
      errors: [
        { pointer: '/username', code: 'required' },
        { pointer: '/email', code: 'invalid_email' },
      ],
    })

    expect(translateApiErrorFields(t, error)).toEqual([
      { field: 'username', message: 'This field is required.' },
      { field: 'email', message: 'Enter a valid email address.' },
    ])
  })
})

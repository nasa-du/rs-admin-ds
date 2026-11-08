import { describe, expect, it } from 'vitest'
import { changePasswordRedirectTarget, sanitizeRedirect } from './redirect'

describe('sanitizeRedirect', () => {
  it('keeps in-site paths with query and hash', () => {
    expect(sanitizeRedirect('/users')).toBe('/users')
    expect(sanitizeRedirect('/users/42?tab=roles#top')).toBe(
      '/users/42?tab=roles#top'
    )
    expect(sanitizeRedirect('/settings/account')).toBe('/settings/account')
  })

  it('falls back to the dashboard for missing or non-path values', () => {
    expect(sanitizeRedirect(undefined)).toBe('/')
    expect(sanitizeRedirect(null)).toBe('/')
    expect(sanitizeRedirect('')).toBe('/')
    expect(sanitizeRedirect('users')).toBe('/')
    expect(sanitizeRedirect('https://evil.example/users')).toBe('/')
    expect(sanitizeRedirect('javascript:alert(1)')).toBe('/')
  })

  it('rejects protocol-relative and backslash host escapes', () => {
    expect(sanitizeRedirect('//evil.example')).toBe('/')
    expect(sanitizeRedirect('/\\evil.example')).toBe('/')
    expect(sanitizeRedirect('/users\\..')).toBe('/')
  })

  it('rejects control characters', () => {
    expect(sanitizeRedirect('/users\n')).toBe('/')
    expect(sanitizeRedirect('/users\u0000')).toBe('/')
  })

  it('rejects dot-segment paths that normalize to protocol-relative', () => {
    expect(sanitizeRedirect('/foo/..//evil.example')).toBe('/')
    expect(sanitizeRedirect('/%2e%2e//evil.example')).toBe('/')
  })

  it('does not loop back to sign-in', () => {
    expect(sanitizeRedirect('/sign-in')).toBe('/')
    expect(sanitizeRedirect('/sign-in/')).toBe('/')
    expect(sanitizeRedirect('/sign-in?redirect=/users')).toBe('/')
  })

  it('keeps a legitimate target that merely contains "sign-in"', () => {
    expect(sanitizeRedirect('/sign-in-history')).toBe('/sign-in-history')
    expect(sanitizeRedirect('/settings/sign-in')).toBe('/settings/sign-in')
  })
})

describe('changePasswordRedirectTarget', () => {
  it('returns an ordinary target unchanged', () => {
    expect(changePasswordRedirectTarget('/users?page=2')).toBe('/users?page=2')
    expect(changePasswordRedirectTarget(undefined)).toBe('/')
  })

  it('avoids rendering change-password again after a successful change', () => {
    expect(changePasswordRedirectTarget('/change-password')).toBe('/')
    expect(changePasswordRedirectTarget('/change-password/')).toBe('/')
    expect(
      changePasswordRedirectTarget('/change-password?redirect=/users')
    ).toBe('/')
  })
})

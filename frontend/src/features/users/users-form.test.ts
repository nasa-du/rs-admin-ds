import { describe, expect, it } from 'vitest'
import type { UserSummary } from '@/api/generated/model'
import {
  buildUserUpdate,
  normalizeEmail,
  normalizeUsername,
} from './users-form'

const USER: UserSummary = {
  id: '0190a1b2-0000-7000-8000-000000000001',
  username: 'alice',
  email: 'alice@example.com',
  display_name: 'Alice',
  phone: '+15551234567',
  status: 'active',
  roles: [],
  must_change_password: false,
  locked_until: null,
  last_login_at: null,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
}

const UNCHANGED = {
  displayName: 'Alice',
  username: 'alice',
  email: 'alice@example.com',
  phone: '+15551234567',
}

describe('normalizeUsername / normalizeEmail', () => {
  it('trims and lowercases a username', () => {
    expect(normalizeUsername('  Alice_01  ')).toBe('alice_01')
  })

  it('trims, normalizes and lowercases an email', () => {
    expect(normalizeEmail('  ALICE@EXAMPLE.COM  ')).toBe('alice@example.com')
    expect(normalizeEmail('éxample@example.com')).toBe('éxample@example.com')
  })
})

describe('buildUserUpdate', () => {
  it('returns null when nothing changed after normalization', () => {
    expect(
      buildUserUpdate(USER, {
        displayName: '  Alice  ',
        username: '  ALICE ',
        email: ' ALICE@EXAMPLE.COM ',
        phone: ' +15551234567 ',
      })
    ).toBeNull()
  })

  it('includes only changed fields', () => {
    expect(
      buildUserUpdate(USER, { ...UNCHANGED, displayName: 'Alice B.' })
    ).toEqual({ display_name: 'Alice B.' })
  })

  it('clears phone with an explicit null', () => {
    expect(buildUserUpdate(USER, { ...UNCHANGED, phone: '   ' })).toEqual({
      phone: null,
    })
  })

  it('normalizes identifiers before comparing and sending', () => {
    expect(
      buildUserUpdate(USER, {
        ...UNCHANGED,
        username: '  Bob ',
        email: ' BOB@EXAMPLE.COM ',
      })
    ).toEqual({ username: 'bob', email: 'bob@example.com' })
  })
})

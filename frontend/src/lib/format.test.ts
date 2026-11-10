import i18n from '@/i18n'
import { afterEach, describe, expect, it } from 'vitest'
import {
  formatAuditTimestamp,
  formatCurrency,
  formatDate,
  formatNumber,
  formatPercent,
  formatTimezoneLabel,
  parseDateTimeLocalValue,
  rfc3339Utc,
  toDateTimeLocalValue,
} from './format'

const date = new Date(Date.UTC(2026, 0, 15, 12, 30))

describe('Intl formatting', () => {
  afterEach(async () => {
    await i18n.changeLanguage('en-US')
  })

  it('formats dates per locale', () => {
    const zh = formatDate(date, { locale: 'zh-CN' })
    const en = formatDate(date, { locale: 'en-US' })

    expect(zh).toContain('2026')
    expect(zh).not.toBe(en)
    expect(en).toBe(
      new Intl.DateTimeFormat('en-US', { dateStyle: 'medium' }).format(date)
    )
  })

  it('formats numbers per locale', () => {
    expect(formatNumber(1234567.89, { locale: 'en-US' })).toBe(
      new Intl.NumberFormat('en-US').format(1234567.89)
    )
  })

  it('formats currency per locale', () => {
    const zh = formatCurrency(1234.5, 'USD', { locale: 'zh-CN' })
    const en = formatCurrency(1234.5, 'USD', { locale: 'en-US' })

    expect(zh).not.toBe(en)
    expect(en).toContain('$')
  })

  it('formats percentages per locale', () => {
    expect(formatPercent(0.256, { locale: 'en-US' })).toBe('26%')
    expect(formatPercent(0.256, { locale: 'zh-CN' })).toBe('26%')
  })

  it('uses the active i18n locale by default', async () => {
    await i18n.changeLanguage('zh-CN')
    expect(formatDate(date)).not.toBe(
      new Intl.DateTimeFormat('en-US', { dateStyle: 'medium' }).format(date)
    )

    await i18n.changeLanguage('en-US')
    expect(formatDate(date)).toBe(
      new Intl.DateTimeFormat('en-US', { dateStyle: 'medium' }).format(date)
    )
  })

  it('returns an empty string for invalid dates', () => {
    expect(formatDate('not-a-date')).toBe('')
  })
})

describe('audit timestamp helpers', () => {
  it('formats with seconds for audit precision', () => {
    expect(formatAuditTimestamp('2026-11-10T12:34:56.000Z', 'en-US')).toMatch(
      /\d{1,2}:\d{2}:\d{2}/
    )
  })

  it('falls back to the raw value when it cannot be parsed', () => {
    expect(formatAuditTimestamp('not-a-date', 'en-US')).toBe('not-a-date')
  })

  it('renders RFC3339 UTC for copying', () => {
    expect(rfc3339Utc('2026-11-10T12:34:56+08:00')).toBe(
      '2026-11-10T04:34:56.000Z'
    )
    expect(rfc3339Utc('not-a-date')).toBe('not-a-date')
  })

  it('renders the browser offset as a stable label', () => {
    expect(formatTimezoneLabel(new Date('2026-01-01T00:00:00Z'))).toMatch(
      /^UTC[+-]\d{2}:\d{2}$/
    )
  })
})

describe('datetime-local conversion', () => {
  it('round-trips an instant through the browser time zone', () => {
    const iso = '2026-11-10T12:34:56.000Z'
    const local = toDateTimeLocalValue(iso)
    expect(local).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/)
    expect(toDateTimeLocalValue(parseDateTimeLocalValue(local)!)).toBe(local)
  })

  it('returns null for empty or invalid input', () => {
    expect(parseDateTimeLocalValue('')).toBeNull()
    expect(parseDateTimeLocalValue('nope')).toBeNull()
    expect(toDateTimeLocalValue('nope')).toBe('')
  })
})

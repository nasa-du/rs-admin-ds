import i18n from '@/i18n'
import { afterEach, describe, expect, it } from 'vitest'
import {
  formatCurrency,
  formatDate,
  formatNumber,
  formatPercent,
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

import { clearCookies } from '@/test-utils/cookies'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getCookie, setCookie } from '@/lib/cookies'
import i18n, {
  resolveInitialLocale,
  setLocale,
  SUPPORTED_LOCALES,
} from './index'

describe('i18n', () => {
  beforeEach(async () => {
    clearCookies()
    await i18n.changeLanguage('zh-CN')
  })

  afterEach(async () => {
    vi.unstubAllGlobals()
    clearCookies()
    await i18n.changeLanguage('en-US')
  })

  it('ships exactly zh-CN and en-US resources', () => {
    expect(SUPPORTED_LOCALES).toEqual(['zh-CN', 'en-US'])
    expect(i18n.hasResourceBundle('zh-CN', 'common')).toBe(true)
    expect(i18n.hasResourceBundle('en-US', 'common')).toBe(true)
    expect(i18n.hasResourceBundle('zh-CN', 'errors')).toBe(true)
    expect(i18n.hasResourceBundle('en-US', 'errors')).toBe(true)
  })

  it('prefers a supported locale cookie', () => {
    setCookie('locale', 'en-US')
    vi.stubGlobal('navigator', {
      languages: ['zh-CN'],
      language: 'zh-CN',
    })

    expect(resolveInitialLocale()).toBe('en-US')
  })

  it('ignores an unsupported locale cookie', () => {
    setCookie('locale', 'fr-FR')
    vi.stubGlobal('navigator', {
      languages: ['zh-CN'],
      language: 'zh-CN',
    })

    expect(resolveInitialLocale()).toBe('zh-CN')
  })

  it('falls back to the first supported browser language', () => {
    vi.stubGlobal('navigator', {
      languages: ['fr-FR', 'en-GB'],
      language: 'fr-FR',
    })

    expect(resolveInitialLocale()).toBe('en-US')
  })

  it('defaults to zh-CN when no supported language is available', () => {
    vi.stubGlobal('navigator', {
      languages: ['fr-FR'],
      language: 'fr-FR',
    })

    expect(resolveInitialLocale()).toBe('zh-CN')
  })

  it('changes language and persists the locale cookie', async () => {
    await setLocale('en-US')

    expect(i18n.resolvedLanguage).toBe('en-US')
    expect(getCookie('locale')).toBe('en-US')

    await setLocale('zh-CN')

    expect(i18n.resolvedLanguage).toBe('zh-CN')
    expect(getCookie('locale')).toBe('zh-CN')
  })

  it('rejects unsupported locales', async () => {
    await expect(setLocale('fr-FR' as never)).rejects.toThrow(
      'Unsupported locale: fr-FR'
    )
  })

  it('throws on a missing key outside production', () => {
    expect(() => i18n.t('common:missingKeyForTest' as never)).toThrow(
      /Missing i18n key/
    )
  })
})

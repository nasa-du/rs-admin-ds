import i18n, { type InitOptions } from 'i18next'
import { initReactI18next } from 'react-i18next'
import { getCookie, setCookie } from '@/lib/cookies'
import enUSaudit from './locales/en-US/audit.json'
import enUSauth from './locales/en-US/auth.json'
import enUScommon from './locales/en-US/common.json'
import enUSerrors from './locales/en-US/errors.json'
import enUSnav from './locales/en-US/nav.json'
import enUSroles from './locales/en-US/roles.json'
import enUSsettings from './locales/en-US/settings.json'
import enUSusers from './locales/en-US/users.json'
import enUSvalidation from './locales/en-US/validation.json'
import zhCNaudit from './locales/zh-CN/audit.json'
import zhCNauth from './locales/zh-CN/auth.json'
import zhCNcommon from './locales/zh-CN/common.json'
import zhCNerrors from './locales/zh-CN/errors.json'
import zhCNnav from './locales/zh-CN/nav.json'
import zhCNroles from './locales/zh-CN/roles.json'
import zhCNsettings from './locales/zh-CN/settings.json'
import zhCNusers from './locales/zh-CN/users.json'
import zhCNvalidation from './locales/zh-CN/validation.json'

export const SUPPORTED_LOCALES = ['zh-CN', 'en-US'] as const
export type Locale = (typeof SUPPORTED_LOCALES)[number]

const DEFAULT_LOCALE: Locale = 'zh-CN'
const FALLBACK_LOCALE: Locale = 'en-US'

const LOCALE_COOKIE_NAME = 'locale'
const LOCALE_COOKIE_MAX_AGE = 60 * 60 * 24 * 365 // 1 year

const NAMESPACES = [
  'common',
  'nav',
  'auth',
  'users',
  'roles',
  'audit',
  'settings',
  'errors',
  'validation',
] as const

const resources = {
  'zh-CN': {
    common: zhCNcommon,
    nav: zhCNnav,
    auth: zhCNauth,
    users: zhCNusers,
    roles: zhCNroles,
    audit: zhCNaudit,
    settings: zhCNsettings,
    errors: zhCNerrors,
    validation: zhCNvalidation,
  },
  'en-US': {
    common: enUScommon,
    nav: enUSnav,
    auth: enUSauth,
    users: enUSusers,
    roles: enUSroles,
    audit: enUSaudit,
    settings: enUSsettings,
    errors: enUSerrors,
    validation: enUSvalidation,
  },
} as const

export function isSupportedLocale(value: string): value is Locale {
  return (SUPPORTED_LOCALES as readonly string[]).includes(value)
}

function localeFromLanguageTag(language: string | undefined): Locale | null {
  if (!language) return null
  const normalized = language.toLowerCase()
  if (normalized.startsWith('zh')) return 'zh-CN'
  if (normalized.startsWith('en')) return 'en-US'
  return null
}

/**
 * Locale resolution order:
 * 1. the `locale` cookie, when it holds a supported value
 * 2. the first supported language in `navigator.languages`
 * 3. zh-CN
 */
export function resolveInitialLocale(): Locale {
  const cookieLocale = getCookie(LOCALE_COOKIE_NAME)
  if (cookieLocale && isSupportedLocale(cookieLocale)) return cookieLocale

  if (typeof navigator !== 'undefined') {
    const languages = navigator.languages?.length
      ? navigator.languages
      : [navigator.language]

    for (const language of languages) {
      const locale = localeFromLanguageTag(language)
      if (locale) return locale
    }
  }

  return DEFAULT_LOCALE
}

export async function setLocale(locale: Locale): Promise<void> {
  if (!isSupportedLocale(locale)) {
    throw new Error(`Unsupported locale: ${locale}`)
  }

  await i18n.changeLanguage(locale)
  setCookie(LOCALE_COOKIE_NAME, locale, {
    maxAge: LOCALE_COOKIE_MAX_AGE,
    sameSite: 'Lax',
  })
}

function missingKeyHandler(
  languages: readonly string[],
  namespace: string,
  key: string
) {
  if (import.meta.env.PROD) return

  throw new Error(
    `Missing i18n key "${key}" in namespace "${namespace}" for ${languages.join(', ')}`
  )
}

const baseOptions = (locale: Locale): InitOptions => ({
  resources,
  lng: locale,
  fallbackLng: FALLBACK_LOCALE,
  supportedLngs: [...SUPPORTED_LOCALES],
  load: 'currentOnly',
  defaultNS: 'common',
  ns: [...NAMESPACES],
  initAsync: false,
  interpolation: { escapeValue: false },
  returnNull: false,
  returnEmptyString: false,
  saveMissing: !import.meta.env.PROD,
  missingKeyHandler,
})

/**
 * Creates a standalone i18n instance (without React bindings). Tests use this
 * so they can run against zh-CN and switch to en-US without mutating the app's
 * shared instance.
 */
export function createI18nInstance(locale: Locale = DEFAULT_LOCALE) {
  const instance = i18n.createInstance()
  void instance.init(baseOptions(locale))
  return instance
}

void i18n.use(initReactI18next).init(baseOptions(resolveInitialLocale()))

export default i18n

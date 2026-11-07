import { type Locale } from './index'

/**
 * Static mapping from locale codes to endonym translation keys.
 * Endonyms are shown in their own language in the language picker.
 */
export const LANGUAGE_LABEL_KEYS = {
  'zh-CN': 'language.zh-CN',
  'en-US': 'language.en-US',
} as const satisfies Record<Locale, string>

import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'

/**
 * Keeps `<html lang>` and the document direction in sync with i18next.
 *
 * v1 is LTR-only: both supported locales map to `ltr`, so the direction is
 * fixed here and the DirectionProvider keeps Radix in sync.
 */
export function LocaleProvider({ children }: { children: React.ReactNode }) {
  const { i18n } = useTranslation()

  useEffect(() => {
    const applyLocale = () => {
      const locale = i18n.resolvedLanguage ?? i18n.language
      document.documentElement.lang = locale
      document.documentElement.dir = 'ltr'
    }

    applyLocale()
    i18n.on('languageChanged', applyLocale)

    return () => i18n.off('languageChanged', applyLocale)
  }, [i18n])

  return <>{children}</>
}

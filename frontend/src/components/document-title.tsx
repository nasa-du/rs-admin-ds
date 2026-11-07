import { useEffect } from 'react'
import { useMatches } from '@tanstack/react-router'
import { type PageTitleKey } from '@/i18n/page-titles'
import { useTranslation } from 'react-i18next'

type RouteStaticData = {
  titleKey?: PageTitleKey
}

/**
 * Updates `document.title` from the deepest matched route that declares a
 * `staticData.titleKey`, and re-renders when the language changes.
 */
export function DocumentTitle() {
  const { t } = useTranslation('common')
  const matches = useMatches()

  const titleKey = [...matches]
    .reverse()
    .map((match) => (match.staticData as RouteStaticData | undefined)?.titleKey)
    .find((key) => key !== undefined)

  // `titleKey` points into many namespaces, so the common-namespace translator
  // is widened here in exactly one place instead of casting at every call site.
  const translateTitle = t as unknown as (key: PageTitleKey) => string

  const appName = t('appName')
  const title = titleKey ? translateTitle(titleKey) : appName

  useEffect(() => {
    document.title = title === appName ? appName : `${title} · ${appName}`
  }, [title, appName])

  return null
}

import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { Settings } from '@/features/settings'

export const Route = createFileRoute('/_authenticated/settings')({
  component: Settings,
  staticData: { titleKey: PAGE_TITLE_KEYS.settings },
})

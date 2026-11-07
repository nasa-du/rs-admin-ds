import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { Dashboard } from '@/features/dashboard'

export const Route = createFileRoute('/_authenticated/')({
  component: Dashboard,
  staticData: { titleKey: PAGE_TITLE_KEYS.dashboard },
})

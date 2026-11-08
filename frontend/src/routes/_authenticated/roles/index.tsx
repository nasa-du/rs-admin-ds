import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { Roles } from '@/features/roles'

export const Route = createFileRoute('/_authenticated/roles/')({
  component: Roles,
  staticData: { titleKey: PAGE_TITLE_KEYS.roles },
})

import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { Users } from '@/features/users'

export const Route = createFileRoute('/_authenticated/users/')({
  component: Users,
  staticData: { titleKey: PAGE_TITLE_KEYS.users },
})

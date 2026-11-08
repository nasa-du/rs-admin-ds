import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { RoleCreate } from '@/features/roles'

export const Route = createFileRoute('/_authenticated/roles/new')({
  component: RoleCreate,
  staticData: { titleKey: PAGE_TITLE_KEYS.roleNew },
})

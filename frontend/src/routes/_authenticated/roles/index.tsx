import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { Roles } from '@/features/roles'
import { rolesSearchSchema } from '@/features/roles/roles-search'

export const Route = createFileRoute('/_authenticated/roles/')({
  component: Roles,
  validateSearch: rolesSearchSchema,
  staticData: { titleKey: PAGE_TITLE_KEYS.roles },
})

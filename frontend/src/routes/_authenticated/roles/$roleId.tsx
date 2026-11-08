import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { RoleDetail } from '@/features/roles'

export const Route = createFileRoute('/_authenticated/roles/$roleId')({
  component: RoleDetail,
  staticData: { titleKey: PAGE_TITLE_KEYS.roleDetail },
})

import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { ForbiddenError } from '@/features/errors/forbidden'

export const Route = createFileRoute('/(errors)/403')({
  component: ForbiddenError,
  staticData: { titleKey: PAGE_TITLE_KEYS.forbidden },
})

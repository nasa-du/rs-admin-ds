import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { UnauthorisedError } from '@/features/errors/unauthorized-error'

export const Route = createFileRoute('/(errors)/401')({
  component: UnauthorisedError,
  staticData: { titleKey: PAGE_TITLE_KEYS.unauthorized },
})

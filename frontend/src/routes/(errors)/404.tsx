import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { NotFoundError } from '@/features/errors/not-found-error'

export const Route = createFileRoute('/(errors)/404')({
  component: NotFoundError,
  staticData: { titleKey: PAGE_TITLE_KEYS.notFound },
})

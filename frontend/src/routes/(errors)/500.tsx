import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { GeneralError } from '@/features/errors/general-error'

export const Route = createFileRoute('/(errors)/500')({
  component: GeneralError,
  staticData: { titleKey: PAGE_TITLE_KEYS.generalError },
})

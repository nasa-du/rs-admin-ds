import { z } from 'zod'
import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { ForbiddenError } from '@/features/errors/forbidden'

const searchSchema = z.object({
  from: z.string().optional(),
})

export const Route = createFileRoute('/(errors)/403')({
  component: ForbiddenError,
  validateSearch: searchSchema,
  staticData: { titleKey: PAGE_TITLE_KEYS.forbidden },
})

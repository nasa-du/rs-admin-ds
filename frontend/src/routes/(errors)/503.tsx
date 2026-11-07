import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { MaintenanceError } from '@/features/errors/maintenance-error'

export const Route = createFileRoute('/(errors)/503')({
  component: MaintenanceError,
  staticData: { titleKey: PAGE_TITLE_KEYS.maintenance },
})

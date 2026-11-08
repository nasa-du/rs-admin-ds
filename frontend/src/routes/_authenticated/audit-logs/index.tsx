import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { AuditLogs } from '@/features/audit-logs'

export const Route = createFileRoute('/_authenticated/audit-logs/')({
  component: AuditLogs,
  staticData: { titleKey: PAGE_TITLE_KEYS.auditLogs },
})

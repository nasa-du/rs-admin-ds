import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { AuditLogs } from '@/features/audit-logs'
import { auditLogsSearchSchema } from '@/features/audit-logs/audit-logs-search'

export const Route = createFileRoute('/_authenticated/audit-logs/')({
  component: AuditLogs,
  validateSearch: auditLogsSearchSchema,
  staticData: { titleKey: PAGE_TITLE_KEYS.auditLogs },
})

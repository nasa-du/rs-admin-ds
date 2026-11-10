import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'
import { Badge } from '@/components/ui/badge'
import {
  auditActionLabel,
  auditResultLabel,
  auditSeverityLabel,
} from '../audit-catalog'

const SEVERITY_CLASSES: Record<string, string> = {
  info: 'border-transparent bg-secondary text-secondary-foreground',
  warning:
    'border-amber-500/40 bg-amber-500/10 text-amber-700 dark:text-amber-400',
  error: 'border-transparent bg-destructive text-white dark:bg-destructive/60',
  critical:
    'border-transparent bg-destructive text-white ring-1 ring-destructive/40 dark:bg-destructive/60',
}

const RESULT_CLASSES: Record<string, string> = {
  success:
    'border-emerald-500/40 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400',
  failure: 'border-destructive/40 text-destructive',
  denied:
    'border-amber-500/40 bg-amber-500/10 text-amber-700 dark:text-amber-400',
}

export function AuditSeverityBadge({ severity }: { severity: string }) {
  const { t } = useTranslation('audit')

  return (
    <Badge
      variant='outline'
      className={cn(SEVERITY_CLASSES[severity])}
      title={severity}
    >
      {auditSeverityLabel(t, severity)}
    </Badge>
  )
}

export function AuditResultBadge({ result }: { result: string }) {
  const { t } = useTranslation('audit')

  return (
    <Badge
      variant='outline'
      className={cn(RESULT_CLASSES[result])}
      title={result}
    >
      {auditResultLabel(t, result)}
    </Badge>
  )
}

/**
 * Localized action label plus the stable code. An unknown action renders the
 * raw code only, never a fabricated label (spec §9.10).
 */
export function AuditActionLabel({ action }: { action: string }) {
  const { t } = useTranslation('audit')
  const label = auditActionLabel(t, action)

  return (
    <span className='inline-flex max-w-full flex-wrap items-center gap-1'>
      {label !== action && <span className='truncate'>{label}</span>}
      <code className='rounded bg-muted px-1 py-0.5 font-mono text-[11px] text-muted-foreground'>
        {action}
      </code>
    </span>
  )
}

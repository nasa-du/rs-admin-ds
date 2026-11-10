import { useMemo, useState } from 'react'
import { Copy } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { formatAuditTimestamp, formatNumber, rfc3339Utc } from '@/lib/format'
import { isUuid } from '@/lib/uuid'
import { Button } from '@/components/ui/button'
import { auditDetailFieldLabel } from '../audit-catalog'
import { maskSensitiveDetails, maskedDetailsJson } from '../audit-mask'
import { AuditCopyButton } from './audit-copy-button'

const ROLE_ID_FIELDS = new Set([
  'role_id',
  'role_ids',
  'added_role_ids',
  'removed_role_ids',
])

const COLLAPSE_THRESHOLD = 3

type AuditDetailsViewProps = {
  details: Record<string, unknown>
  canLinkRoles: boolean
  onOpenRole: (roleId: string) => void
}

/**
 * Structured/raw view of an audit `details` object (spec §9.10). Both views
 * render the masked copy; the raw view only pretty-prints it.
 */
export function AuditDetailsView({
  details,
  canLinkRoles,
  onOpenRole,
}: AuditDetailsViewProps) {
  const { t } = useTranslation('audit')
  const [view, setView] = useState<'structured' | 'raw'>('structured')

  const masked = useMemo(
    () => maskSensitiveDetails(details) as Record<string, unknown>,
    [details]
  )
  const entries = useMemo(() => Object.entries(masked), [masked])

  return (
    <div className='grid gap-3'>
      <div className='flex items-center gap-1' role='tablist'>
        <Button
          id='audit-details-tab-structured'
          type='button'
          role='tab'
          aria-selected={view === 'structured'}
          aria-controls='audit-details-panel'
          variant={view === 'structured' ? 'secondary' : 'ghost'}
          size='sm'
          onClick={() => setView('structured')}
        >
          {t('detail.structured')}
        </Button>
        <Button
          id='audit-details-tab-raw'
          type='button'
          role='tab'
          aria-selected={view === 'raw'}
          aria-controls='audit-details-panel'
          variant={view === 'raw' ? 'secondary' : 'ghost'}
          size='sm'
          onClick={() => setView('raw')}
        >
          {t('detail.raw')}
        </Button>
      </div>

      <div
        id='audit-details-panel'
        role='tabpanel'
        aria-labelledby={`audit-details-tab-${view}`}
      >
        {view === 'structured' ? (
          entries.length > 0 ? (
            <StructuredEntries
              entries={entries}
              canLinkRoles={canLinkRoles}
              onOpenRole={onOpenRole}
            />
          ) : (
            <p className='text-sm text-muted-foreground'>
              {t('detail.noDetails')}
            </p>
          )
        ) : (
          <RawJsonView details={masked} />
        )}
      </div>
    </div>
  )
}

function StructuredEntries({
  entries,
  canLinkRoles,
  onOpenRole,
}: {
  entries: Array<[string, unknown]>
  canLinkRoles: boolean
  onOpenRole: (roleId: string) => void
}) {
  const { t } = useTranslation('audit')

  return (
    <dl className='grid gap-3'>
      {entries.map(([key, value]) => (
        <div key={key} className='grid gap-1 sm:grid-cols-[10rem_1fr] sm:gap-3'>
          <dt className='text-xs font-medium text-muted-foreground'>
            {auditDetailFieldLabel(t, key)}
          </dt>
          <dd className='min-w-0 text-sm'>
            <StructuredValue
              fieldKey={key}
              value={value}
              canLinkRoles={canLinkRoles}
              onOpenRole={onOpenRole}
            />
          </dd>
        </div>
      ))}
    </dl>
  )
}

function StructuredValue({
  fieldKey,
  value,
  canLinkRoles,
  onOpenRole,
}: {
  fieldKey: string
  value: unknown
  canLinkRoles: boolean
  onOpenRole: (roleId: string) => void
}) {
  if (value === null || value === undefined) {
    return <span className='text-muted-foreground'>—</span>
  }

  if (typeof value === 'boolean' || typeof value === 'number') {
    return (
      <span className='font-mono text-xs'>
        {typeof value === 'number' ? formatNumber(value) : String(value)}
      </span>
    )
  }

  if (typeof value === 'string') {
    return (
      <StringValue
        fieldKey={fieldKey}
        value={value}
        canLinkRoles={canLinkRoles}
        onOpenRole={onOpenRole}
      />
    )
  }

  if (Array.isArray(value)) {
    if (value.length === 0) {
      return <span className='text-muted-foreground'>—</span>
    }

    if (value.every((item) => typeof item === 'string')) {
      return (
        <StringList
          fieldKey={fieldKey}
          values={value as string[]}
          canLinkRoles={canLinkRoles}
          onOpenRole={onOpenRole}
        />
      )
    }

    return (
      <CollapsibleItems
        items={value}
        render={(item, index) =>
          item !== null && typeof item === 'object' ? (
            <div className='rounded-md border p-2'>
              <StructuredEntries
                entries={Object.entries(item)}
                canLinkRoles={canLinkRoles}
                onOpenRole={onOpenRole}
              />
            </div>
          ) : (
            <span className='font-mono text-xs'>
              {index}: {String(item)}
            </span>
          )
        }
      />
    )
  }

  if (typeof value === 'object') {
    return (
      <div className='rounded-md border p-2'>
        <StructuredEntries
          entries={Object.entries(value)}
          canLinkRoles={canLinkRoles}
          onOpenRole={onOpenRole}
        />
      </div>
    )
  }

  return <span className='break-words'>{String(value)}</span>
}

function StringValue({
  fieldKey,
  value,
  canLinkRoles,
  onOpenRole,
}: {
  fieldKey: string
  value: string
  canLinkRoles: boolean
  onOpenRole: (roleId: string) => void
}) {
  if (isUuid(value)) {
    if (canLinkRoles && ROLE_ID_FIELDS.has(fieldKey)) {
      return (
        <button
          type='button'
          className='font-mono text-xs break-all text-primary underline-offset-4 hover:underline'
          onClick={() => onOpenRole(value)}
        >
          {value}
        </button>
      )
    }

    return <span className='font-mono text-xs break-all'>{value}</span>
  }

  if (looksLikeTimestamp(fieldKey, value)) {
    return <span title={rfc3339Utc(value)}>{formatAuditTimestamp(value)}</span>
  }

  return <span className='break-words whitespace-pre-wrap'>{value}</span>
}

function StringList({
  fieldKey,
  values,
  canLinkRoles,
  onOpenRole,
}: {
  fieldKey: string
  values: string[]
  canLinkRoles: boolean
  onOpenRole: (roleId: string) => void
}) {
  const [expanded, setExpanded] = useState(false)
  const { t } = useTranslation('audit')
  const visible = expanded ? values : values.slice(0, COLLAPSE_THRESHOLD)

  return (
    <div className='grid gap-1'>
      <div className='flex flex-wrap gap-1'>
        {visible.map((value, index) => {
          const isRoleId =
            canLinkRoles && ROLE_ID_FIELDS.has(fieldKey) && isUuid(value)

          return isRoleId ? (
            <button
              key={`${value}-${index}`}
              type='button'
              className='rounded-md border px-1.5 py-0.5 font-mono text-[11px] text-primary underline-offset-4 hover:underline'
              onClick={() => onOpenRole(value)}
            >
              {value}
            </button>
          ) : (
            <span
              key={`${value}-${index}`}
              className='rounded-md bg-muted px-1.5 py-0.5 font-mono text-[11px] break-all'
            >
              {value}
            </span>
          )
        })}
      </div>
      {values.length > COLLAPSE_THRESHOLD && (
        <Button
          type='button'
          variant='ghost'
          size='sm'
          className='h-6 w-fit px-2 text-xs'
          onClick={() => setExpanded((current) => !current)}
        >
          {expanded
            ? t('detail.showLess')
            : t('detail.showMore', {
                count: values.length - COLLAPSE_THRESHOLD,
              })}
        </Button>
      )}
    </div>
  )
}

function CollapsibleItems({
  items,
  render,
}: {
  items: unknown[]
  render: (item: unknown, index: number) => React.ReactNode
}) {
  const { t } = useTranslation('audit')
  const [expanded, setExpanded] = useState(false)
  const visible = expanded ? items : items.slice(0, COLLAPSE_THRESHOLD)

  return (
    <div className='grid gap-1'>
      {visible.map((item, index) => (
        <div key={index}>{render(item, index)}</div>
      ))}
      {items.length > COLLAPSE_THRESHOLD && (
        <Button
          type='button'
          variant='ghost'
          size='sm'
          className='h-6 w-fit px-2 text-xs'
          onClick={() => setExpanded((current) => !current)}
        >
          {expanded
            ? t('detail.showLess')
            : t('detail.showMore', {
                count: items.length - COLLAPSE_THRESHOLD,
              })}
        </Button>
      )}
    </div>
  )
}

function RawJsonView({ details }: { details: Record<string, unknown> }) {
  const { t } = useTranslation('audit')
  const json = useMemo(() => maskedDetailsJson(details), [details])

  return (
    <div className='grid gap-2'>
      <pre className='max-h-96 overflow-auto rounded-md border bg-muted/50 p-3 font-mono text-xs break-all whitespace-pre-wrap'>
        {json}
      </pre>
      <AuditCopyButton
        value={json}
        label={t('detail.copyJson')}
        variant='outline'
        size='sm'
        className='w-fit'
      >
        <Copy className='size-3.5' />
        {t('detail.copyJson')}
      </AuditCopyButton>
    </div>
  )
}

function looksLikeTimestamp(fieldKey: string, value: string): boolean {
  if (fieldKey.endsWith('_at') || fieldKey === 'auth_time') return true
  return /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/.test(value)
}

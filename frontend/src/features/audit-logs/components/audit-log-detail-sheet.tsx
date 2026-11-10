import { type ErrorTranslator, translateApiError } from '@/i18n/errors'
import { Filter, FileQuestion } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { AuditLogDetail } from '@/api/generated/model'
import { useGetAuditLog } from '@/api/generated/rs-admin'
import { toApiError } from '@/lib/api-error'
import {
  formatAuditTimestamp,
  formatTimezoneLabel,
  rfc3339Utc,
} from '@/lib/format'
import { Button } from '@/components/ui/button'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { Skeleton } from '@/components/ui/skeleton'
import { ApiErrorAlert } from '@/components/api-error-alert'
import { auditActorLabel, auditTargetTypeLabel } from '../audit-catalog'
import type { AuditLogsSearch } from '../audit-logs-search'
import {
  AuditActionLabel,
  AuditResultBadge,
  AuditSeverityBadge,
} from './audit-badges'
import { AuditCopyButton } from './audit-copy-button'
import { AuditDetailsView } from './audit-details-view'

type AuditLogDetailSheetProps = {
  detailId: string | undefined
  onClose: () => void
  onFilter: (patch: Partial<AuditLogsSearch>) => void
  canReadUsers: boolean
  canReadRoles: boolean
  onOpenUser: (userId: string) => void
  onOpenRole: (roleId: string) => void
}

type AuditObjectLinks = {
  canReadUsers: boolean
  canReadRoles: boolean
  onOpenUser: (userId: string) => void
  onOpenRole: (roleId: string) => void
}

/**
 * URL-bound audit detail Sheet (spec §9.10). The GET always runs when the
 * Sheet opens (no cache, no prefetch) so every view writes an
 * `audit_log_accessed` event on the server.
 */
export function AuditLogDetailSheet({
  detailId,
  onClose,
  onFilter,
  canReadUsers,
  canReadRoles,
  onOpenUser,
  onOpenRole,
}: AuditLogDetailSheetProps) {
  const { t } = useTranslation('audit')
  const { t: tCommon } = useTranslation('common')
  const { t: tErrors } = useTranslation('errors')
  const timezone = formatTimezoneLabel()
  const objectLinks: AuditObjectLinks = {
    canReadUsers,
    canReadRoles,
    onOpenUser,
    onOpenRole,
  }

  const query = useGetAuditLog(detailId ?? '', {
    query: {
      enabled: Boolean(detailId),
      staleTime: 0,
      gcTime: 0,
      refetchOnMount: 'always',
      // Each detail GET writes `audit_log_accessed`; do not re-fetch on focus
      // or reconnect while the Sheet stays open (spec §9.10).
      refetchOnWindowFocus: false,
      refetchOnReconnect: false,
    },
  })

  if (!detailId) return null

  const detail = query.data
  const apiError = toApiError(query.error)
  const isMissing = apiError?.code === 'not_found'

  return (
    <Sheet
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <SheetContent className='w-full overflow-y-auto sm:max-w-2xl'>
        <SheetHeader>
          <SheetTitle>{t('detail.title')}</SheetTitle>
          <SheetDescription>{t('detail.description')}</SheetDescription>
        </SheetHeader>

        {query.isPending && !detail ? (
          <div className='grid gap-3 px-4'>
            {Array.from({ length: 8 }, (_, index) => (
              <Skeleton key={index} className='h-5 w-full' />
            ))}
          </div>
        ) : isMissing ? (
          <div className='grid justify-items-center gap-2 px-4 py-10 text-center'>
            <FileQuestion className='size-8 text-muted-foreground' />
            <p className='font-medium'>{t('detail.empty.title')}</p>
            <p className='max-w-md text-sm text-muted-foreground'>
              {t('detail.empty.description')}
            </p>
          </div>
        ) : query.isError ? (
          <div className='grid gap-3 px-4'>
            <ApiErrorAlert
              error={query.error}
              message={translateApiError(
                tErrors as ErrorTranslator,
                query.error
              )}
            />
            <Button
              type='button'
              variant='outline'
              onClick={() => void query.refetch()}
            >
              {tCommon('error.retry')}
            </Button>
          </div>
        ) : detail ? (
          <div className='grid gap-6 px-4'>
            <section className='grid gap-2'>
              <h3 className='text-sm font-semibold'>{t('detail.summary')}</h3>
              <dl className='grid gap-1'>
                <DetailRow label={t('detail.time')}>
                  <span className='inline-flex items-center gap-1'>
                    <span title={rfc3339Utc(detail.occurred_at)}>
                      {formatAuditTimestamp(detail.occurred_at)}
                    </span>
                    <AuditCopyButton
                      value={rfc3339Utc(detail.occurred_at)}
                      label={t('detail.copyTime')}
                      className='size-6'
                    />
                  </span>
                </DetailRow>
                <DetailRow label={t('detail.severity')}>
                  <AuditSeverityBadge severity={detail.severity} />
                </DetailRow>
                <DetailRow label={t('detail.action')}>
                  <AuditActionLabel action={detail.action} />
                </DetailRow>
                <DetailRow label={t('detail.result')}>
                  <AuditResultBadge result={detail.result} />
                </DetailRow>
                <DetailRow label={t('detail.actor')}>
                  <ActorValue detail={detail} links={objectLinks} />
                </DetailRow>
                <DetailRow label={t('detail.target')}>
                  <TargetValue detail={detail} links={objectLinks} />
                </DetailRow>
                <DetailRow label={t('detail.ip')}>
                  {detail.ip ? (
                    <span className='font-mono text-xs'>{detail.ip}</span>
                  ) : (
                    <span className='text-muted-foreground'>—</span>
                  )}
                </DetailRow>
                <DetailRow label={t('detail.userAgent')}>
                  {detail.user_agent ? (
                    <span className='break-words'>{detail.user_agent}</span>
                  ) : (
                    <span className='text-muted-foreground'>—</span>
                  )}
                </DetailRow>
                <DetailRow label={t('detail.requestId')}>
                  <span className='inline-flex items-center gap-1'>
                    <span className='font-mono text-xs break-all'>
                      {detail.request_id}
                    </span>
                    <AuditCopyButton
                      value={detail.request_id}
                      label={t('detail.copyRequestId')}
                      className='size-6'
                    />
                  </span>
                </DetailRow>
                <DetailRow label={t('detail.id')}>
                  <span className='inline-flex items-center gap-1'>
                    <span className='font-mono text-xs break-all'>
                      {detail.id}
                    </span>
                    <AuditCopyButton
                      value={detail.id}
                      label={t('detail.copyId')}
                      className='size-6'
                    />
                  </span>
                </DetailRow>
              </dl>
              <p className='text-xs text-muted-foreground'>
                {t('detail.timezoneNote', { timezone })}
              </p>
            </section>

            <section className='grid gap-2'>
              <h3 className='text-sm font-semibold'>{t('detail.details')}</h3>
              <AuditDetailsView
                details={detail.details}
                canLinkRoles={canReadRoles}
                onOpenRole={onOpenRole}
              />
            </section>

            <section className='flex flex-wrap gap-2 border-t pt-4'>
              {detail.actor.id && (
                <Button
                  type='button'
                  variant='outline'
                  size='sm'
                  onClick={() =>
                    onFilter({ actor_id: detail.actor.id ?? undefined })
                  }
                >
                  <Filter className='size-3.5' />
                  {t('detail.filterActor')}
                </Button>
              )}
              {detail.target.type && detail.target.id && (
                <Button
                  type='button'
                  variant='outline'
                  size='sm'
                  onClick={() =>
                    onFilter({
                      target_type: detail.target.type,
                      target_id: detail.target.id ?? undefined,
                    })
                  }
                >
                  <Filter className='size-3.5' />
                  {t('detail.filterTarget')}
                </Button>
              )}
              <Button
                type='button'
                variant='outline'
                size='sm'
                onClick={() => onFilter({ request_id: detail.request_id })}
              >
                <Filter className='size-3.5' />
                {t('detail.filterRequest')}
              </Button>
            </section>
          </div>
        ) : null}
      </SheetContent>
    </Sheet>
  )
}

function ActorValue({
  detail,
  links,
}: {
  detail: AuditLogDetail
  links: AuditObjectLinks
}) {
  const { t } = useTranslation('audit')
  const label = auditActorLabel(t, detail.actor)

  return (
    <span className='grid gap-1'>
      <span>{label}</span>
      {detail.actor.id && (
        <span className='inline-flex flex-wrap items-center gap-1'>
          <span className='font-mono text-xs break-all'>{detail.actor.id}</span>
          <AuditCopyButton
            value={detail.actor.id}
            label={t('detail.copyActorId')}
            className='size-6'
          />
          {links.canReadUsers && (
            <Button
              type='button'
              variant='link'
              size='sm'
              className='h-auto p-0 text-xs'
              onClick={() => links.onOpenUser(detail.actor.id!)}
            >
              {t('detail.openActor')}
            </Button>
          )}
        </span>
      )}
    </span>
  )
}

function TargetValue({
  detail,
  links,
}: {
  detail: AuditLogDetail
  links: AuditObjectLinks
}) {
  const { t } = useTranslation('audit')
  const { type, id, label } = detail.target
  const canOpenUser = detail.target.type === 'user' && links.canReadUsers && id
  const canOpenRole = detail.target.type === 'role' && links.canReadRoles && id

  return (
    <span className='grid gap-1'>
      <span>{auditTargetTypeLabel(t, type)}</span>
      {(label || id) && (
        <span className='inline-flex flex-wrap items-center gap-1'>
          <span
            className={label ? 'break-words' : 'font-mono text-xs break-all'}
          >
            {label ?? id}
          </span>
          {id && (
            <AuditCopyButton
              value={id}
              label={t('detail.copyTargetId')}
              className='size-6'
            />
          )}
          {canOpenUser && (
            <Button
              type='button'
              variant='link'
              size='sm'
              className='h-auto p-0 text-xs'
              onClick={() => links.onOpenUser(id!)}
            >
              {t('detail.openTarget')}
            </Button>
          )}
          {canOpenRole && (
            <Button
              type='button'
              variant='link'
              size='sm'
              className='h-auto p-0 text-xs'
              onClick={() => links.onOpenRole(id!)}
            >
              {t('detail.openTarget')}
            </Button>
          )}
        </span>
      )}
    </span>
  )
}

function DetailRow({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className='grid grid-cols-[7.5rem_1fr] gap-2 py-1 text-sm'>
      <dt className='text-muted-foreground'>{label}</dt>
      <dd className='min-w-0 break-words'>{children}</dd>
    </div>
  )
}

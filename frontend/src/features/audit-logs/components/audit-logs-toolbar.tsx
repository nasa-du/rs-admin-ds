import { useEffect } from 'react'
import type { Table } from '@tanstack/react-table'
import { ListFilter, RefreshCw, Search, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { AuditLogSummary } from '@/api/generated/model'
import { formatTimezoneLabel } from '@/lib/format'
import { isUuid } from '@/lib/uuid'
import { useSyncedValue } from '@/hooks/use-synced-value'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { DataTableViewOptions } from '@/components/data-table/view-options'
import {
  AUDIT_ACTION_GROUPS,
  AUDIT_RESULTS,
  AUDIT_SEVERITIES,
  AUDIT_TARGET_TYPES,
  auditActionGroupLabel,
  auditActionLabel,
  auditResultLabel,
  auditSeverityLabel,
  auditTargetTypeLabel,
  isKnownAuditAction,
  isKnownAuditTargetType,
  type KnownAuditResult,
  type KnownAuditSeverity,
} from '../audit-catalog'
import {
  clearFiltersPatch,
  isIpAddress,
  type AuditLogsSearch,
} from '../audit-logs-search'
import type { AuditWindow } from '../audit-time'
import { AuditActorFilter } from './audit-actor-filter'
import { AuditTimeRangeFilter } from './audit-time-range-filter'

type AuditLogsToolbarProps = {
  table: Table<AuditLogSummary>
  search: AuditLogsSearch
  auditWindow: AuditWindow
  canReadUsers: boolean
  isFetching: boolean
  onListChange: (patch: Partial<AuditLogsSearch>) => void
  onRefresh: () => void
}

/**
 * Filter row for the audit table (spec §9.10). Selects apply immediately,
 * text filters are debounced by 300ms, and every change resets the page and
 * closes the detail Sheet through `onListChange`.
 */
export function AuditLogsToolbar({
  table,
  search,
  auditWindow,
  canReadUsers,
  isFetching,
  onListChange,
  onRefresh,
}: AuditLogsToolbarProps) {
  const { t } = useTranslation('audit')
  const timezone = formatTimezoneLabel()

  const unknownAction =
    search.action && !isKnownAuditAction(search.action)
      ? search.action
      : undefined
  const unknownTargetType =
    search.target_type && !isKnownAuditTargetType(search.target_type)
      ? search.target_type
      : undefined

  const chipLabel = (label: string, value: string) =>
    t('filters.chipLabel', { label, value })
  const chips: Array<{ key: string; label: string; remove: () => void }> = []

  if (search.actor_id) {
    chips.push({
      key: 'actor',
      label: chipLabel(t('filters.actor'), search.actor_id),
      remove: () => onListChange({ actor_id: undefined }),
    })
  }
  if (search.action) {
    chips.push({
      key: 'action',
      label: chipLabel(t('filters.action'), auditActionLabel(t, search.action)),
      remove: () => onListChange({ action: undefined }),
    })
  }
  if (search.severity) {
    chips.push({
      key: 'severity',
      label: chipLabel(
        t('filters.severity'),
        auditSeverityLabel(t, search.severity)
      ),
      remove: () => onListChange({ severity: undefined }),
    })
  }
  if (search.result) {
    chips.push({
      key: 'result',
      label: chipLabel(t('filters.result'), auditResultLabel(t, search.result)),
      remove: () => onListChange({ result: undefined }),
    })
  }
  if (search.target_type) {
    chips.push({
      key: 'target-type',
      label: chipLabel(
        t('filters.targetType'),
        auditTargetTypeLabel(t, search.target_type)
      ),
      remove: () => onListChange({ target_type: undefined }),
    })
  }
  if (search.target_id) {
    chips.push({
      key: 'target-id',
      label: chipLabel(t('filters.targetId'), search.target_id),
      remove: () => onListChange({ target_id: undefined }),
    })
  }
  if (search.request_id) {
    chips.push({
      key: 'request-id',
      label: chipLabel(t('filters.requestId'), search.request_id),
      remove: () => onListChange({ request_id: undefined }),
    })
  }
  if (search.ip) {
    chips.push({
      key: 'ip',
      label: chipLabel(t('filters.ip'), search.ip),
      remove: () => onListChange({ ip: undefined }),
    })
  }

  const moreFilterCount = [
    search.target_type,
    search.target_id,
    search.request_id,
    search.ip,
  ].filter(Boolean).length

  return (
    <div className='flex flex-col gap-2'>
      <div className='flex flex-wrap items-center gap-2'>
        <AuditTimeRangeFilter
          search={search}
          auditWindow={auditWindow}
          onChange={onListChange}
        />

        <AuditActorFilter
          actorId={search.actor_id}
          canReadUsers={canReadUsers}
          onChange={(actorId) => onListChange({ actor_id: actorId })}
        />

        <Select
          value={search.action ?? 'all'}
          onValueChange={(value) =>
            onListChange({
              action: value === 'all' ? undefined : value,
            })
          }
        >
          <SelectTrigger
            size='sm'
            className='h-8 w-44'
            aria-label={t('filters.action')}
          >
            <SelectValue placeholder={t('filters.allActions')} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value='all'>{t('filters.allActions')}</SelectItem>
            {unknownAction && (
              <SelectItem value={unknownAction}>{unknownAction}</SelectItem>
            )}
            {AUDIT_ACTION_GROUPS.map((group) => (
              <SelectGroup key={group.id}>
                <SelectLabel>{auditActionGroupLabel(t, group.id)}</SelectLabel>
                {group.actions.map((action) => (
                  <SelectItem key={action} value={action}>
                    {auditActionLabel(t, action)}
                  </SelectItem>
                ))}
              </SelectGroup>
            ))}
          </SelectContent>
        </Select>

        <Select
          value={search.severity ?? 'all'}
          onValueChange={(value) =>
            onListChange({
              severity:
                value === 'all' ? undefined : (value as KnownAuditSeverity),
            })
          }
        >
          <SelectTrigger
            size='sm'
            className='h-8 w-32'
            aria-label={t('filters.severity')}
          >
            <SelectValue placeholder={t('filters.allSeverities')} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value='all'>{t('filters.allSeverities')}</SelectItem>
            {AUDIT_SEVERITIES.map((severity) => (
              <SelectItem key={severity} value={severity}>
                {auditSeverityLabel(t, severity)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <Select
          value={search.result ?? 'all'}
          onValueChange={(value) =>
            onListChange({
              result: value === 'all' ? undefined : (value as KnownAuditResult),
            })
          }
        >
          <SelectTrigger
            size='sm'
            className='h-8 w-32'
            aria-label={t('filters.result')}
          >
            <SelectValue placeholder={t('filters.allResults')} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value='all'>{t('filters.allResults')}</SelectItem>
            {AUDIT_RESULTS.map((result) => (
              <SelectItem key={result} value={result}>
                {auditResultLabel(t, result)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <Popover>
          <PopoverTrigger asChild>
            <Button
              type='button'
              variant='outline'
              size='sm'
              className='h-8 border-dashed'
            >
              <ListFilter className='size-4' />
              {t('filters.more')}
              {moreFilterCount > 0 && (
                <Badge variant='secondary'>{moreFilterCount}</Badge>
              )}
            </Button>
          </PopoverTrigger>
          <PopoverContent align='start' className='grid w-72 gap-3'>
            <div className='grid gap-1.5'>
              <Label>{t('filters.targetType')}</Label>
              <Select
                value={search.target_type ?? 'all'}
                onValueChange={(value) =>
                  onListChange({
                    target_type: value === 'all' ? undefined : value,
                  })
                }
              >
                <SelectTrigger size='sm' aria-label={t('filters.targetType')}>
                  <SelectValue placeholder={t('filters.allTargetTypes')} />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value='all'>
                    {t('filters.allTargetTypes')}
                  </SelectItem>
                  {unknownTargetType && (
                    <SelectItem value={unknownTargetType}>
                      {unknownTargetType}
                    </SelectItem>
                  )}
                  {AUDIT_TARGET_TYPES.map((type) => (
                    <SelectItem key={type} value={type}>
                      {auditTargetTypeLabel(t, type)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <DebouncedInputFilter
              id='audit-filter-target-id'
              label={t('filters.targetId')}
              value={search.target_id}
              validate={isUuid}
              placeholder={t('filters.targetIdPlaceholder')}
              onApply={(value) => onListChange({ target_id: value })}
            />
            <DebouncedInputFilter
              id='audit-filter-request-id'
              label={t('filters.requestId')}
              value={search.request_id}
              validate={isUuid}
              placeholder={t('filters.requestIdPlaceholder')}
              onApply={(value) => onListChange({ request_id: value })}
            />
            <DebouncedInputFilter
              id='audit-filter-ip'
              label={t('filters.ip')}
              value={search.ip}
              validate={isIpAddress}
              invalidMessage={t('filters.ipInvalid')}
              placeholder={t('filters.ipPlaceholder')}
              onApply={(value) => onListChange({ ip: value })}
            />
          </PopoverContent>
        </Popover>

        <div className='ms-auto flex items-center gap-2'>
          <Button
            type='button'
            variant='outline'
            size='sm'
            className='h-8'
            onClick={onRefresh}
            disabled={isFetching}
            aria-label={t('filters.refresh')}
          >
            <RefreshCw
              className={isFetching ? 'size-4 animate-spin' : 'size-4'}
            />
            <span className='hidden sm:inline'>{t('filters.refresh')}</span>
          </Button>
          <DataTableViewOptions table={table} />
        </div>
      </div>

      <div className='flex flex-wrap items-center gap-1'>
        {chips.map((chip) => (
          <Badge key={chip.key} variant='secondary' className='gap-1 pe-1'>
            {chip.label}
            <button
              type='button'
              className='rounded-sm p-0.5 hover:bg-background/60'
              aria-label={t('filters.remove', { filter: chip.label })}
              onClick={chip.remove}
            >
              <X className='size-3' />
            </button>
          </Badge>
        ))}
        {chips.length > 0 && (
          <Button
            type='button'
            variant='ghost'
            size='sm'
            className='h-6 px-2 text-xs'
            onClick={() => onListChange(clearFiltersPatch())}
          >
            {t('filters.clearAll')}
          </Button>
        )}
        <span className='ms-auto text-xs text-muted-foreground'>
          {t('filters.timezone', { timezone })}
        </span>
      </div>
    </div>
  )
}

function DebouncedInputFilter({
  id,
  label,
  value,
  validate,
  invalidMessage,
  placeholder,
  onApply,
}: {
  id: string
  label: string
  value: string | undefined
  validate: (value: string) => boolean
  invalidMessage?: string
  placeholder: string
  onApply: (value: string | undefined) => void
}) {
  const [input, setInput] = useSyncedValue(value ?? '')
  const trimmed = input.trim()
  const invalid = trimmed !== '' && !validate(trimmed)

  useEffect(() => {
    if (trimmed === (value ?? '')) return
    if (trimmed !== '' && !validate(trimmed)) return

    const timer = window.setTimeout(() => {
      onApply(trimmed || undefined)
    }, 300)
    return () => window.clearTimeout(timer)
  }, [trimmed, value, validate, onApply])

  return (
    <div className='grid gap-1.5'>
      <Label htmlFor={id}>{label}</Label>
      <div className='relative'>
        <Search className='pointer-events-none absolute inset-s-2 top-1/2 size-4 -translate-y-1/2 text-muted-foreground' />
        <Input
          id={id}
          value={input}
          onChange={(event) => setInput(event.target.value)}
          placeholder={placeholder}
          aria-label={label}
          aria-invalid={invalid}
          className='h-8 ps-8 font-mono text-xs'
        />
      </div>
      {invalid && invalidMessage && (
        <p className='text-xs text-destructive'>{invalidMessage}</p>
      )}
    </div>
  )
}

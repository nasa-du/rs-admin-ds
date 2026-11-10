import { useMemo, useState } from 'react'
import {
  type ColumnDef,
  type PaginationState,
  type SortingState,
  type VisibilityState,
  flexRender,
  getCoreRowModel,
  useReactTable,
} from '@tanstack/react-table'
import { type ErrorTranslator, translateApiError } from '@/i18n/errors'
import { AlertTriangle, ScrollText } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type {
  AuditActorRef,
  AuditLogListResponse,
  AuditLogSummary,
  AuditTargetRef,
} from '@/api/generated/model'
import {
  formatAuditTimestamp,
  formatTimezoneLabel,
  rfc3339Utc,
} from '@/lib/format'
import { cn } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { ApiErrorAlert } from '@/components/api-error-alert'
import { DataTableColumnHeader } from '@/components/data-table/column-header'
import { DataTablePagination } from '@/components/data-table/pagination'
import { auditActorLabel, auditTargetTypeLabel } from '../audit-catalog'
import {
  AUDIT_PAGE_SIZE_DEFAULT,
  AUDIT_PAGE_SIZE_OPTIONS,
  hasActiveFilters,
  sortingFromSearch,
  type AuditLogsSearch,
} from '../audit-logs-search'
import type { AuditWindow } from '../audit-time'
import {
  AuditActionLabel,
  AuditResultBadge,
  AuditSeverityBadge,
} from './audit-badges'
import { AuditCopyButton } from './audit-copy-button'
import { AuditLogsToolbar } from './audit-logs-toolbar'

type AuditLogsTableProps = {
  data: AuditLogListResponse | undefined
  isPending: boolean
  isFetching: boolean
  isError: boolean
  error: unknown
  search: AuditLogsSearch
  auditWindow: AuditWindow
  canReadUsers: boolean
  canReadRoles: boolean
  onListChange: (patch: Partial<AuditLogsSearch>) => void
  onPaginationChange: (next: PaginationState) => void
  onSortingChange: (next: SortingState) => void
  onOpenDetail: (id: string) => void
  onOpenUser: (userId: string) => void
  onOpenRole: (roleId: string) => void
  onRefresh: () => void
  onRetry: () => void
  onClearFilters: () => void
  onResetRange: () => void
}

/**
 * Server-driven audit table (spec §9.10): manual pagination/sorting, skeleton,
 * empty and error states, row click opens the URL-synced detail Sheet, and
 * actor/target identifiers link to their object pages when permitted.
 */
export function AuditLogsTable({
  data,
  isPending,
  isFetching,
  isError,
  error,
  search,
  auditWindow,
  canReadUsers,
  canReadRoles,
  onListChange,
  onPaginationChange,
  onSortingChange,
  onOpenDetail,
  onOpenUser,
  onOpenRole,
  onRefresh,
  onRetry,
  onClearFilters,
  onResetRange,
}: AuditLogsTableProps) {
  const { t } = useTranslation('audit')
  const { t: tCommon } = useTranslation('common')
  const { t: tErrors } = useTranslation('errors')
  const timezone = formatTimezoneLabel()

  const [columnVisibility, setColumnVisibility] = useState<VisibilityState>({
    request_id: false,
  })
  const sorting = useMemo(() => sortingFromSearch(search), [search])

  const pageSize = search.page_size ?? AUDIT_PAGE_SIZE_DEFAULT
  const pageIndex = (search.page ?? 1) - 1
  const total = data?.total ?? 0
  const pageCount = Math.max(1, Math.ceil(total / pageSize))

  const columns = useMemo<ColumnDef<AuditLogSummary>[]>(
    () => [
      {
        accessorKey: 'occurred_at',
        header: ({ column }) => (
          <DataTableColumnHeader
            column={column}
            title={`${t('columns.time')} (${timezone})`}
          />
        ),
        cell: ({ row }) => (
          <span
            className='whitespace-nowrap'
            title={rfc3339Utc(row.original.occurred_at)}
          >
            {formatAuditTimestamp(row.original.occurred_at)}
          </span>
        ),
        meta: { title: t('columns.time') },
        enableSorting: true,
      },
      {
        accessorKey: 'severity',
        header: t('columns.severity'),
        cell: ({ row }) => (
          <AuditSeverityBadge severity={row.original.severity} />
        ),
        meta: { title: t('columns.severity') },
        enableSorting: false,
      },
      {
        accessorKey: 'action',
        header: t('columns.action'),
        cell: ({ row }) => <AuditActionLabel action={row.original.action} />,
        meta: { title: t('columns.action') },
        enableSorting: false,
      },
      {
        id: 'actor',
        accessorFn: (row) => row.actor.label,
        header: t('columns.actor'),
        cell: ({ row }) => (
          <ActorCell
            actor={row.original.actor}
            canReadUsers={canReadUsers}
            onOpenUser={onOpenUser}
          />
        ),
        meta: { title: t('columns.actor') },
        enableSorting: false,
      },
      {
        id: 'target',
        accessorFn: (row) => row.target.label ?? row.target.id ?? '',
        header: t('columns.target'),
        cell: ({ row }) => (
          <TargetCell
            target={row.original.target}
            canReadUsers={canReadUsers}
            canReadRoles={canReadRoles}
            onOpenUser={onOpenUser}
            onOpenRole={onOpenRole}
          />
        ),
        meta: { title: t('columns.target') },
        enableSorting: false,
      },
      {
        accessorKey: 'result',
        header: t('columns.result'),
        cell: ({ row }) => <AuditResultBadge result={row.original.result} />,
        meta: { title: t('columns.result') },
        enableSorting: false,
      },
      {
        accessorKey: 'ip',
        header: t('columns.ip'),
        cell: ({ row }) =>
          row.original.ip ? (
            <span className='font-mono text-xs'>{row.original.ip}</span>
          ) : (
            <span className='text-muted-foreground'>—</span>
          ),
        meta: { title: t('columns.ip') },
        enableSorting: false,
      },
      {
        accessorKey: 'request_id',
        header: t('columns.requestId'),
        cell: ({ row }) => (
          <span
            className='block max-w-40 truncate font-mono text-xs'
            title={row.original.request_id}
          >
            {row.original.request_id}
          </span>
        ),
        meta: { title: t('columns.requestId') },
        enableSorting: false,
      },
    ],
    [t, timezone, canReadUsers, canReadRoles, onOpenUser, onOpenRole]
  )

  const table = useReactTable({
    data: data?.items ?? [],
    columns,
    state: {
      pagination: { pageIndex, pageSize },
      sorting,
      columnVisibility,
    },
    pageCount,
    manualPagination: true,
    manualSorting: true,
    manualFiltering: true,
    enableMultiSort: false,
    enableRowSelection: false,
    onColumnVisibilityChange: setColumnVisibility,
    onPaginationChange: (updater) => {
      const next =
        typeof updater === 'function'
          ? updater({ pageIndex, pageSize })
          : updater
      onPaginationChange(next)
    },
    onSortingChange: (updater) => {
      const next = typeof updater === 'function' ? updater(sorting) : updater
      onSortingChange(next)
    },
    getCoreRowModel: getCoreRowModel(),
  })

  const rows = table.getRowModel().rows
  const visibleColumnCount = table.getVisibleLeafColumns().length
  const showSkeleton = isPending && !data
  const hasErrorWithoutData = isError && !data
  const isEmpty =
    !showSkeleton && !hasErrorWithoutData && data?.items.length === 0
  const filtersActive = hasActiveFilters(search)

  return (
    <div className='flex flex-col gap-4'>
      <AuditLogsToolbar
        table={table}
        search={search}
        auditWindow={auditWindow}
        canReadUsers={canReadUsers}
        isFetching={isFetching}
        onListChange={onListChange}
        onRefresh={onRefresh}
      />

      {isError && data && (
        <div className='grid justify-items-start gap-2'>
          <ApiErrorAlert
            error={error}
            message={translateApiError(tErrors as ErrorTranslator, error)}
          />
          <Button type='button' variant='outline' size='sm' onClick={onRetry}>
            {tCommon('error.retry')}
          </Button>
        </div>
      )}

      {hasErrorWithoutData ? (
        <div className='grid justify-items-start gap-3 rounded-md border p-4'>
          <div className='flex items-center gap-2 text-sm font-medium'>
            <AlertTriangle className='size-4 text-destructive' />
            {t('states.loadError')}
          </div>
          <ApiErrorAlert
            error={error}
            message={translateApiError(tErrors as ErrorTranslator, error)}
          />
          <Button type='button' variant='outline' onClick={onRetry}>
            {tCommon('error.retry')}
          </Button>
        </div>
      ) : (
        <>
          <div className='overflow-hidden rounded-md border'>
            <div className='w-full overflow-x-auto'>
              <Table>
                <TableHeader>
                  {table.getHeaderGroups().map((headerGroup) => (
                    <TableRow key={headerGroup.id}>
                      {headerGroup.headers.map((header) => {
                        const meta = header.column.columnDef.meta
                        return (
                          <TableHead
                            key={header.id}
                            className={cn(meta?.className, meta?.thClassName)}
                          >
                            {header.isPlaceholder
                              ? null
                              : flexRender(
                                  header.column.columnDef.header,
                                  header.getContext()
                                )}
                          </TableHead>
                        )
                      })}
                    </TableRow>
                  ))}
                </TableHeader>
                <TableBody
                  aria-busy={isFetching}
                  className={cn(isFetching && !showSkeleton && 'opacity-60')}
                >
                  {showSkeleton ? (
                    Array.from({ length: 6 }, (_, rowIndex) => (
                      <TableRow key={`skeleton-${rowIndex}`}>
                        {table.getVisibleLeafColumns().map((column) => (
                          <TableCell key={column.id}>
                            <Skeleton className='h-4 w-full' />
                          </TableCell>
                        ))}
                      </TableRow>
                    ))
                  ) : isEmpty ? (
                    <TableRow>
                      <TableCell colSpan={visibleColumnCount} className='p-0'>
                        <EmptyState
                          hasFilters={filtersActive}
                          onClearFilters={onClearFilters}
                          onResetRange={onResetRange}
                        />
                      </TableCell>
                    </TableRow>
                  ) : (
                    rows.map((row) => (
                      <TableRow
                        key={row.id}
                        tabIndex={0}
                        className='cursor-pointer'
                        onClick={() => onOpenDetail(row.original.id)}
                        onKeyDown={(event) => {
                          if (event.target !== event.currentTarget) return
                          if (event.key === 'Enter' || event.key === ' ') {
                            event.preventDefault()
                            onOpenDetail(row.original.id)
                          }
                        }}
                      >
                        {row.getVisibleCells().map((cell) => {
                          const meta = cell.column.columnDef.meta
                          return (
                            <TableCell
                              key={cell.id}
                              className={cn(meta?.className, meta?.tdClassName)}
                            >
                              {flexRender(
                                cell.column.columnDef.cell,
                                cell.getContext()
                              )}
                            </TableCell>
                          )
                        })}
                      </TableRow>
                    ))
                  )}
                </TableBody>
              </Table>
            </div>
          </div>

          {showSkeleton && (
            <span role='status' className='sr-only'>
              {t('states.loading')}
            </span>
          )}

          {data && !showSkeleton && (
            <div className='flex flex-wrap items-center justify-between gap-2'>
              <p className='text-sm text-muted-foreground'>
                {t('filters.total', { count: total })}
              </p>
              {data.total > 0 && (
                <DataTablePagination
                  table={table}
                  pageSizeOptions={AUDIT_PAGE_SIZE_OPTIONS}
                />
              )}
            </div>
          )}
        </>
      )}
    </div>
  )
}

function ActorCell({
  actor,
  canReadUsers,
  onOpenUser,
}: {
  actor: AuditActorRef
  canReadUsers: boolean
  onOpenUser: (userId: string) => void
}) {
  const { t } = useTranslation('audit')

  return (
    <span className='grid max-w-44 gap-0.5'>
      <span className='truncate' title={actor.label}>
        {auditActorLabel(t, actor)}
      </span>
      {actor.id && (
        <span className='inline-flex min-w-0 items-center gap-1'>
          {canReadUsers ? (
            <button
              type='button'
              className='truncate text-start font-mono text-[11px] text-primary underline-offset-4 hover:underline'
              title={actor.id}
              onClick={(event) => {
                event.stopPropagation()
                onOpenUser(actor.id!)
              }}
            >
              {actor.id}
            </button>
          ) : (
            <span
              className='truncate font-mono text-[11px] text-muted-foreground'
              title={actor.id}
            >
              {actor.id}
            </span>
          )}
          <AuditCopyButton
            value={actor.id}
            label={t('columns.copyActorId')}
            stopPropagation
            className='size-5 shrink-0'
          />
        </span>
      )}
    </span>
  )
}

function TargetCell({
  target,
  canReadUsers,
  canReadRoles,
  onOpenUser,
  onOpenRole,
}: {
  target: AuditTargetRef
  canReadUsers: boolean
  canReadRoles: boolean
  onOpenUser: (userId: string) => void
  onOpenRole: (roleId: string) => void
}) {
  const { t } = useTranslation('audit')
  const identifier = target.label ?? target.id
  const canOpenUser =
    target.type === 'user' && Boolean(target.id) && canReadUsers
  const canOpenRole =
    target.type === 'role' && Boolean(target.id) && canReadRoles

  return (
    <span className='grid max-w-44 gap-0.5'>
      <span>{auditTargetTypeLabel(t, target.type)}</span>
      {identifier && (
        <span className='inline-flex min-w-0 items-center gap-1'>
          {canOpenUser ? (
            <button
              type='button'
              className='truncate text-start underline-offset-4 hover:underline'
              title={identifier}
              onClick={(event) => {
                event.stopPropagation()
                onOpenUser(target.id!)
              }}
            >
              {identifier}
            </button>
          ) : canOpenRole ? (
            <button
              type='button'
              className='truncate text-start underline-offset-4 hover:underline'
              title={identifier}
              onClick={(event) => {
                event.stopPropagation()
                onOpenRole(target.id!)
              }}
            >
              {identifier}
            </button>
          ) : (
            <span
              className='truncate text-xs text-muted-foreground'
              title={identifier}
            >
              {identifier}
            </span>
          )}
          {!target.label && target.id && (
            <AuditCopyButton
              value={target.id}
              label={t('columns.copyTargetId')}
              stopPropagation
              className='size-5 shrink-0'
            />
          )}
        </span>
      )}
    </span>
  )
}

function EmptyState({
  hasFilters,
  onClearFilters,
  onResetRange,
}: {
  hasFilters: boolean
  onClearFilters: () => void
  onResetRange: () => void
}) {
  const { t } = useTranslation('audit')

  return (
    <div className='flex flex-col items-center gap-2 px-4 py-12 text-center'>
      <ScrollText className='size-8 text-muted-foreground' />
      <p className='font-medium'>
        {hasFilters ? t('states.noMatches.title') : t('states.noLogs.title')}
      </p>
      <p className='max-w-md text-sm text-muted-foreground'>
        {hasFilters
          ? t('states.noMatches.description')
          : t('states.noLogs.description')}
      </p>
      <div className='flex flex-wrap justify-center gap-2'>
        {hasFilters && (
          <Button
            type='button'
            variant='outline'
            size='sm'
            onClick={onClearFilters}
          >
            {t('states.clearFilters')}
          </Button>
        )}
        <Button
          type='button'
          variant='outline'
          size='sm'
          onClick={onResetRange}
        >
          {t('states.widenRange')}
        </Button>
      </div>
    </div>
  )
}

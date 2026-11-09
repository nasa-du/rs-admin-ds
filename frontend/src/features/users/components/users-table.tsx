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
import {
  AlertTriangle,
  MoreHorizontal,
  Pencil,
  Users as UsersIcon,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type {
  RoleSummary,
  UserListResponse,
  UserSelfResponse,
  UserSummary,
} from '@/api/generated/model'
import { formatDate, formatNumber } from '@/lib/format'
import { hasPermission } from '@/lib/permissions'
import { cn } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { ApiErrorAlert } from '@/components/api-error-alert'
import { DataTableColumnHeader } from '@/components/data-table/column-header'
import { DataTablePagination } from '@/components/data-table/pagination'
import {
  USERS_PAGE_SIZE_DEFAULT,
  USERS_PAGE_SIZE_OPTIONS,
  hasActiveFilters,
  sortingFromSearch,
  type UsersSearch,
} from '../users-search'
import { UserRoleChips } from './user-role-chips'
import { UserStatusBadges } from './user-status-badges'
import { UsersToolbar } from './users-toolbar'

type UsersTableProps = {
  data: UserListResponse | undefined
  isPending: boolean
  isFetching: boolean
  isError: boolean
  error: unknown
  search: UsersSearch
  currentUser: UserSelfResponse | null
  roleOptions: RoleSummary[]
  rolesLoading: boolean
  rolesError: boolean
  onRetryRoles: () => void
  onListChange: (patch: Partial<UsersSearch>) => void
  onPaginationChange: (next: PaginationState) => void
  onSortingChange: (next: SortingState) => void
  onOpenUser: (id: string) => void
  onEditUser: (user: UserSummary) => void
  onRefresh: () => void
  onRetry: () => void
  onClearFilters: () => void
}

/**
 * Server-driven users table (spec §9.8): manual pagination, sorting and
 * filtering, skeleton/empty/error states, and no optimistic updates.
 */
export function UsersTable({
  data,
  isPending,
  isFetching,
  isError,
  error,
  search,
  currentUser,
  roleOptions,
  rolesLoading,
  rolesError,
  onRetryRoles,
  onListChange,
  onPaginationChange,
  onSortingChange,
  onOpenUser,
  onEditUser,
  onRefresh,
  onRetry,
  onClearFilters,
}: UsersTableProps) {
  const { t } = useTranslation('users')
  const { t: tCommon } = useTranslation('common')
  const { t: tErrors } = useTranslation('errors')

  const canUpdate = hasPermission(currentUser, 'users.update')
  const canReadRoles = hasPermission(currentUser, 'roles.read')
  const [columnVisibility, setColumnVisibility] = useState<VisibilityState>({
    phone: false,
    updated_at: false,
  })
  const sorting = useMemo(() => sortingFromSearch(search), [search])

  const pageSize = search.page_size ?? USERS_PAGE_SIZE_DEFAULT
  const pageIndex = (search.page ?? 1) - 1
  const total = data?.total ?? 0
  const pageCount = Math.max(1, Math.ceil(total / pageSize))

  const columns = useMemo<ColumnDef<UserSummary>[]>(() => {
    const base: ColumnDef<UserSummary>[] = [
      {
        accessorKey: 'username',
        header: ({ column }) => (
          <DataTableColumnHeader
            column={column}
            title={t('columns.username')}
          />
        ),
        cell: ({ row }) => (
          <span
            className='block max-w-48 truncate font-medium'
            title={row.original.username}
          >
            {row.original.username}
          </span>
        ),
        meta: {
          title: t('columns.username'),
          className: 'sticky start-0 z-10 bg-background',
        },
        enableHiding: false,
        enableSorting: true,
      },
      {
        accessorKey: 'display_name',
        header: ({ column }) => (
          <DataTableColumnHeader
            column={column}
            title={t('columns.displayName')}
          />
        ),
        cell: ({ row }) => (
          <span
            className='block max-w-48 truncate'
            title={row.original.display_name}
          >
            {row.original.display_name}
          </span>
        ),
        meta: { title: t('columns.displayName') },
        enableSorting: true,
      },
      {
        accessorKey: 'email',
        header: ({ column }) => (
          <DataTableColumnHeader column={column} title={t('columns.email')} />
        ),
        cell: ({ row }) => (
          <span className='block max-w-56 truncate' title={row.original.email}>
            {row.original.email}
          </span>
        ),
        meta: { title: t('columns.email') },
        enableSorting: true,
      },
      {
        accessorKey: 'phone',
        header: t('columns.phone'),
        cell: ({ row }) => row.original.phone ?? '—',
        meta: { title: t('columns.phone') },
        enableSorting: false,
      },
      {
        id: 'roles',
        accessorFn: (row) => row.roles,
        header: t('columns.roles'),
        cell: ({ row }) => <UserRoleChips roles={row.original.roles} />,
        meta: { title: t('columns.roles') },
        enableSorting: false,
      },
      {
        accessorKey: 'status',
        header: ({ column }) => (
          <DataTableColumnHeader column={column} title={t('columns.status')} />
        ),
        cell: ({ row }) => (
          <UserStatusBadges
            status={row.original.status}
            lockedUntil={row.original.locked_until}
            mustChangePassword={row.original.must_change_password}
          />
        ),
        meta: { title: t('columns.status') },
        enableSorting: true,
      },
      {
        accessorKey: 'last_login_at',
        header: ({ column }) => (
          <DataTableColumnHeader
            column={column}
            title={t('columns.lastLoginAt')}
          />
        ),
        cell: ({ row }) =>
          row.original.last_login_at ? (
            <span title={row.original.last_login_at}>
              {formatDate(row.original.last_login_at, { preset: 'dateTime' })}
            </span>
          ) : (
            <span className='text-muted-foreground'>
              {t('status.neverSignedIn')}
            </span>
          ),
        meta: { title: t('columns.lastLoginAt') },
        enableSorting: true,
      },
      {
        accessorKey: 'created_at',
        header: ({ column }) => (
          <DataTableColumnHeader
            column={column}
            title={t('columns.createdAt')}
          />
        ),
        cell: ({ row }) => (
          <span title={row.original.created_at}>
            {formatDate(row.original.created_at, { preset: 'dateTime' })}
          </span>
        ),
        meta: { title: t('columns.createdAt') },
        enableSorting: true,
      },
      {
        accessorKey: 'updated_at',
        header: ({ column }) => (
          <DataTableColumnHeader
            column={column}
            title={t('columns.updatedAt')}
          />
        ),
        cell: ({ row }) => (
          <span title={row.original.updated_at}>
            {formatDate(row.original.updated_at, { preset: 'dateTime' })}
          </span>
        ),
        meta: { title: t('columns.updatedAt') },
        enableSorting: true,
      },
    ]

    if (canUpdate) {
      base.push({
        id: 'actions',
        header: t('columns.actions'),
        cell: ({ row }) => (
          <UserRowActions
            user={row.original}
            currentUser={currentUser}
            onEdit={onEditUser}
          />
        ),
        meta: { title: t('columns.actions') },
        enableSorting: false,
        enableHiding: false,
      })
    }

    return base
  }, [t, canUpdate, currentUser, onEditUser])

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
      <UsersToolbar
        table={table}
        search={search}
        roleOptions={roleOptions}
        canReadRoles={canReadRoles}
        rolesLoading={rolesLoading}
        rolesError={rolesError}
        onRetryRoles={onRetryRoles}
        onListChange={onListChange}
        onRefresh={onRefresh}
        isFetching={isFetching}
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
                    Array.from({ length: 5 }, (_, rowIndex) => (
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
                        />
                      </TableCell>
                    </TableRow>
                  ) : (
                    rows.map((row) => (
                      <TableRow
                        key={row.id}
                        tabIndex={0}
                        className='cursor-pointer'
                        onClick={() => onOpenUser(row.original.id)}
                        onKeyDown={(event) => {
                          if (event.target !== event.currentTarget) return
                          if (event.key === 'Enter' || event.key === ' ') {
                            event.preventDefault()
                            onOpenUser(row.original.id)
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
                {t('filters.total', {
                  count: total,
                  total: formatNumber(total),
                })}
              </p>
              {data.total > 0 && (
                <DataTablePagination
                  table={table}
                  pageSizeOptions={USERS_PAGE_SIZE_OPTIONS}
                />
              )}
            </div>
          )}
        </>
      )}
    </div>
  )
}

function UserRowActions({
  user,
  currentUser,
  onEdit,
}: {
  user: UserSummary
  currentUser: UserSelfResponse | null
  onEdit: (user: UserSummary) => void
}) {
  const { t } = useTranslation('users')
  const isSelf = currentUser?.id === user.id

  const editItem = (
    <DropdownMenuItem disabled={isSelf} onSelect={() => onEdit(user)}>
      <Pencil className='size-4' />
      {t('actions.edit')}
    </DropdownMenuItem>
  )

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button
          type='button'
          variant='ghost'
          size='icon'
          className='size-8'
          aria-label={t('actions.more')}
          onClick={(event) => event.stopPropagation()}
        >
          <MoreHorizontal className='size-4' />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align='end'
        onClick={(event) => event.stopPropagation()}
      >
        {isSelf ? (
          <TooltipProvider delayDuration={0}>
            <Tooltip>
              <TooltipTrigger asChild>
                <span className='block'>{editItem}</span>
              </TooltipTrigger>
              <TooltipContent side='left'>{t('edit.self')}</TooltipContent>
            </Tooltip>
          </TooltipProvider>
        ) : (
          editItem
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function EmptyState({
  hasFilters,
  onClearFilters,
}: {
  hasFilters: boolean
  onClearFilters: () => void
}) {
  const { t } = useTranslation('users')

  return (
    <div className='flex flex-col items-center gap-2 px-4 py-12 text-center'>
      <UsersIcon className='size-8 text-muted-foreground' />
      <p className='font-medium'>
        {hasFilters ? t('states.noMatches.title') : t('states.noUsers.title')}
      </p>
      <p className='max-w-md text-sm text-muted-foreground'>
        {hasFilters
          ? t('states.noMatches.description')
          : t('states.noUsers.description')}
      </p>
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
    </div>
  )
}

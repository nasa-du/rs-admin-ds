import { useCallback, useEffect, useMemo, useState } from 'react'
import { keepPreviousData } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
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
import { AlertTriangle, ListFilter, Plus } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type {
  RoleListResponse,
  RoleSummary,
  UserSelfResponse,
} from '@/api/generated/model'
import { useListRoles } from '@/api/generated/rs-admin'
import { formatDate, formatNumber } from '@/lib/format'
import { hasPermission } from '@/lib/permissions'
import { roleDisplayName } from '@/lib/role-labels'
import { cn } from '@/lib/utils'
import { Badge } from '@/components/ui/badge'
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
import { ConfigDrawer } from '@/components/config-drawer'
import { DataTableColumnHeader } from '@/components/data-table/column-header'
import { DataTablePagination } from '@/components/data-table/pagination'
import { Header } from '@/components/layout/header'
import { Main } from '@/components/layout/main'
import { ProfileDropdown } from '@/components/profile-dropdown'
import { Search } from '@/components/search'
import { ThemeSwitch } from '@/components/theme-switch'
import { Can } from '@/features/auth/can'
import { useAuth } from '@/features/auth/use-auth'
import { RoleDeleteDialog } from './components/role-delete-dialog'
import { RoleRowActions } from './components/role-row-actions'
import { RolesToolbar } from './components/roles-toolbar'
import {
  ROLES_PAGE_SIZE_DEFAULT,
  ROLES_PAGE_SIZE_OPTIONS,
  clearFiltersPatch,
  hasActiveFilters,
  listFiltersPatch,
  paginationPatch,
  sortingFromSearch,
  sortingPatch,
  toListRolesParams,
  type RolesSearch,
} from './roles-search'

/**
 * Roles list page (spec §9.9): server-side search, type filter, sorting and
 * pagination synced to the URL; no optimistic updates.
 */
export function Roles() {
  const { t } = useTranslation('roles')
  const search = useSearch({ from: '/_authenticated/roles/' })
  const navigate = useNavigate()
  const currentUser = useAuth()

  const pageSize = search.page_size ?? ROLES_PAGE_SIZE_DEFAULT
  const [deletingRole, setDeletingRole] = useState<RoleSummary | null>(null)

  const params = useMemo(() => toListRolesParams(search), [search])
  const listQuery = useListRoles(params, {
    query: { placeholderData: keepPreviousData },
  })

  const handleListChange = useCallback(
    (patch: Partial<RolesSearch>) => {
      void navigate({
        to: '/roles',
        search: (previous) => ({ ...previous, ...listFiltersPatch(patch) }),
        replace: true,
      })
    },
    [navigate]
  )

  const handlePaginationChange = useCallback(
    (next: PaginationState) => {
      void navigate({
        to: '/roles',
        search: (previous) => ({
          ...previous,
          ...paginationPatch(next, pageSize),
        }),
        replace: true,
      })
    },
    [navigate, pageSize]
  )

  const handleSortingChange = useCallback(
    (next: SortingState) => {
      void navigate({
        to: '/roles',
        search: (previous) => ({ ...previous, ...sortingPatch(next) }),
        replace: true,
      })
    },
    [navigate]
  )

  const openRole = useCallback(
    (id: string) => {
      void navigate({ to: '/roles/$roleId', params: { roleId: id } })
    },
    [navigate]
  )

  const clearFilters = useCallback(() => {
    void navigate({
      to: '/roles',
      search: (previous) => ({ ...previous, ...clearFiltersPatch() }),
      replace: true,
    })
  }, [navigate])

  useEffect(() => {
    if (!listQuery.data || listQuery.isPlaceholderData) return

    const pageCount = Math.max(1, Math.ceil(listQuery.data.total / pageSize))
    const currentPage = search.page ?? 1
    if (currentPage <= pageCount) return

    void navigate({
      to: '/roles',
      search: (previous) => ({
        ...previous,
        page: pageCount > 1 ? pageCount : undefined,
      }),
      replace: true,
    })
  }, [
    listQuery.data,
    listQuery.isPlaceholderData,
    pageSize,
    search.page,
    navigate,
  ])

  return (
    <>
      <Header fixed>
        <Search className='me-auto' />
        <ThemeSwitch />
        <ConfigDrawer />
        <ProfileDropdown />
      </Header>

      <Main className='flex flex-1 flex-col gap-4 sm:gap-6'>
        <div className='flex flex-wrap items-end justify-between gap-3'>
          <div>
            <h2 className='text-2xl font-bold tracking-tight'>{t('title')}</h2>
            <p className='text-muted-foreground'>{t('description')}</p>
          </div>
          <Can permission='roles.create'>
            <Button
              type='button'
              onClick={() => void navigate({ to: '/roles/new' })}
            >
              <Plus className='size-4' />
              {t('actions.create')}
            </Button>
          </Can>
        </div>

        <RolesTable
          data={listQuery.data}
          isPending={listQuery.isPending}
          isFetching={listQuery.isFetching}
          isError={listQuery.isError}
          error={listQuery.error}
          search={search}
          currentUser={currentUser}
          onListChange={handleListChange}
          onPaginationChange={handlePaginationChange}
          onSortingChange={handleSortingChange}
          onOpenRole={openRole}
          onDeleteRole={setDeletingRole}
          onRefresh={() => void listQuery.refetch()}
          onRetry={() => void listQuery.refetch()}
          onClearFilters={clearFilters}
        />
      </Main>

      {deletingRole && (
        <RoleDeleteDialog
          open
          role={deletingRole}
          currentUser={currentUser}
          onOpenChange={(open) => {
            if (!open) setDeletingRole(null)
          }}
          onDeleted={() => {
            setDeletingRole(null)
            void listQuery.refetch()
          }}
        />
      )}
    </>
  )
}

type RoleType = 'builtin' | 'custom' | 'readOnly'

function roleType(role: RoleSummary): RoleType {
  if (role.is_builtin) return 'builtin'
  return role.manageable ? 'custom' : 'readOnly'
}

type RolesTableProps = {
  data: RoleListResponse | undefined
  isPending: boolean
  isFetching: boolean
  isError: boolean
  error: unknown
  search: RolesSearch
  currentUser: UserSelfResponse | null
  onListChange: (patch: Partial<RolesSearch>) => void
  onPaginationChange: (next: PaginationState) => void
  onSortingChange: (next: SortingState) => void
  onOpenRole: (id: string) => void
  onDeleteRole: (role: RoleSummary) => void
  onRefresh: () => void
  onRetry: () => void
  onClearFilters: () => void
}

function RolesTable({
  data,
  isPending,
  isFetching,
  isError,
  error,
  search,
  currentUser,
  onListChange,
  onPaginationChange,
  onSortingChange,
  onOpenRole,
  onDeleteRole,
  onRefresh,
  onRetry,
  onClearFilters,
}: RolesTableProps) {
  const { t } = useTranslation('roles')
  const { t: tCommon } = useTranslation('common')
  const { t: tErrors } = useTranslation('errors')

  const canUpdate = hasPermission(currentUser, 'roles.update')
  const canDelete = hasPermission(currentUser, 'roles.delete')
  const [columnVisibility, setColumnVisibility] = useState<VisibilityState>({
    description: true,
  })
  const sorting = useMemo(() => sortingFromSearch(search), [search])

  const pageSize = search.page_size ?? ROLES_PAGE_SIZE_DEFAULT
  const pageIndex = (search.page ?? 1) - 1
  const total = data?.total ?? 0
  const pageCount = Math.max(1, Math.ceil(total / pageSize))

  const columns = useMemo<ColumnDef<RoleSummary>[]>(() => {
    const base: ColumnDef<RoleSummary>[] = [
      {
        accessorKey: 'name',
        header: ({ column }) => (
          <DataTableColumnHeader column={column} title={t('columns.name')} />
        ),
        cell: ({ row }) => (
          <div className='flex flex-wrap items-center gap-1.5'>
            <span className='font-medium'>
              {roleDisplayName(t, row.original)}
            </span>
            {row.original.is_builtin && (
              <>
                <Badge variant='secondary'>{t('type.builtin')}</Badge>
                <Badge variant='outline' className='font-mono text-[10px]'>
                  {row.original.name}
                </Badge>
              </>
            )}
          </div>
        ),
        meta: {
          title: t('columns.name'),
          className: 'sticky start-0 z-10 bg-background',
        },
        enableHiding: false,
        enableSorting: true,
      },
      {
        accessorKey: 'description',
        header: t('columns.description'),
        cell: ({ row }) => (
          <span
            className='block max-w-64 truncate text-muted-foreground'
            title={row.original.description ?? undefined}
          >
            {row.original.description ?? '—'}
          </span>
        ),
        meta: { title: t('columns.description') },
        enableSorting: false,
      },
      {
        id: 'permissions',
        accessorFn: (row) => row.permission_keys.length,
        header: t('columns.permissions'),
        cell: ({ row }) =>
          t('list.permissionCount', {
            count: row.original.permission_keys.length,
          }),
        meta: { title: t('columns.permissions') },
        enableSorting: false,
      },
      {
        id: 'type',
        accessorFn: (row) => roleType(row),
        header: t('columns.type'),
        cell: ({ row }) => {
          const type = roleType(row.original)
          return (
            <Badge variant={type === 'builtin' ? 'outline' : 'secondary'}>
              {type === 'builtin'
                ? t('type.builtin')
                : type === 'custom'
                  ? t('type.custom')
                  : t('type.readOnly')}
            </Badge>
          )
        },
        meta: { title: t('columns.type') },
        enableSorting: false,
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

    base.push({
      id: 'actions',
      header: t('columns.actions'),
      cell: ({ row }) => (
        <RoleRowActions
          role={row.original}
          canUpdate={canUpdate}
          canDelete={canDelete}
          onOpenRole={onOpenRole}
          onDeleteRole={onDeleteRole}
        />
      ),
      meta: { title: t('columns.actions') },
      enableSorting: false,
      enableHiding: false,
    })

    return base
  }, [t, canUpdate, canDelete, onOpenRole, onDeleteRole])

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
      <RolesToolbar
        table={table}
        search={search}
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
                        onClick={() => onOpenRole(row.original.id)}
                        onKeyDown={(event) => {
                          if (event.target !== event.currentTarget) return
                          if (event.key === 'Enter' || event.key === ' ') {
                            event.preventDefault()
                            onOpenRole(row.original.id)
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
                  pageSizeOptions={ROLES_PAGE_SIZE_OPTIONS}
                />
              )}
            </div>
          )}
        </>
      )}
    </div>
  )
}

function EmptyState({
  hasFilters,
  onClearFilters,
}: {
  hasFilters: boolean
  onClearFilters: () => void
}) {
  const { t } = useTranslation('roles')

  return (
    <div className='flex flex-col items-center gap-2 px-4 py-12 text-center'>
      <ListFilter className='size-8 text-muted-foreground' />
      <p className='font-medium'>
        {hasFilters ? t('states.noMatches.title') : t('states.noRoles.title')}
      </p>
      <p className='max-w-md text-sm text-muted-foreground'>
        {hasFilters
          ? t('states.noMatches.description')
          : t('states.noRoles.description')}
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

import { useCallback, useEffect, useMemo } from 'react'
import { keepPreviousData } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
import type { PaginationState, SortingState } from '@tanstack/react-table'
import { Info } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { useListAuditLogs } from '@/api/generated/rs-admin'
import { hasPermission } from '@/lib/permissions'
import { Button } from '@/components/ui/button'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { ConfigDrawer } from '@/components/config-drawer'
import { Header } from '@/components/layout/header'
import { Main } from '@/components/layout/main'
import { ProfileDropdown } from '@/components/profile-dropdown'
import { Search } from '@/components/search'
import { ThemeSwitch } from '@/components/theme-switch'
import { useAuth } from '@/features/auth/use-auth'
import {
  AUDIT_PAGE_SIZE_DEFAULT,
  clearFiltersPatch,
  listFiltersPatch,
  paginationPatch,
  presetWindowPatch,
  sortingPatch,
  toListAuditLogsParams,
  type AuditLogsSearch,
} from './audit-logs-search'
import { resolveAuditWindow } from './audit-time'
import { AuditLogDetailSheet } from './components/audit-log-detail-sheet'
import { AuditLogsTable } from './components/audit-logs-table'

/**
 * Functional URL-search reducer so concurrent updates (window sync, filters,
 * paging) compose instead of clobbering each other.
 */
function searchWithPatch(patch: Partial<AuditLogsSearch>) {
  return (previous: Record<string, unknown>) =>
    ({ ...previous, ...patch }) as AuditLogsSearch
}

/**
 * Audit log query page (spec §9.10). URL search is the single source of truth
 * for the list and the detail Sheet: the first visit materializes the default
 * 30-day preset into an absolute UTC window, so pagination, refreshes and
 * shared links all query the same range.
 */
export function AuditLogs() {
  const { t } = useTranslation('audit')
  const search = useSearch({ from: '/_authenticated/audit-logs/' })
  const navigate = useNavigate()
  const currentUser = useAuth()

  const canReadUsers = hasPermission(currentUser, 'users.read')
  const canReadRoles = hasPermission(currentUser, 'roles.read')
  const pageSize = search.page_size ?? AUDIT_PAGE_SIZE_DEFAULT

  const { from, to, range } = search
  const auditWindow = useMemo(
    () => resolveAuditWindow({ from, to, range }),
    [from, to, range]
  )
  const needsWindowSync = !search.from || !search.to

  useEffect(() => {
    if (!needsWindowSync) return

    void navigate({
      to: '/audit-logs',
      search: searchWithPatch({
        from: auditWindow.from,
        to: auditWindow.to,
        range: auditWindow.range,
      }),
      replace: true,
    })
  }, [needsWindowSync, auditWindow, navigate])

  const params = useMemo(
    () => toListAuditLogsParams(search, auditWindow),
    [search, auditWindow]
  )
  const listQuery = useListAuditLogs(params, {
    query: { placeholderData: keepPreviousData },
  })

  const handleListChange = useCallback(
    (patch: Partial<AuditLogsSearch>) => {
      void navigate({
        to: '/audit-logs',
        search: searchWithPatch(listFiltersPatch(patch)),
        replace: true,
      })
    },
    [navigate]
  )

  const handlePaginationChange = useCallback(
    (next: PaginationState) => {
      void navigate({
        to: '/audit-logs',
        search: searchWithPatch(paginationPatch(next, pageSize)),
        replace: true,
      })
    },
    [navigate, pageSize]
  )

  const handleSortingChange = useCallback(
    (next: SortingState) => {
      void navigate({
        to: '/audit-logs',
        search: searchWithPatch(sortingPatch(next)),
        replace: true,
      })
    },
    [navigate]
  )

  const openDetail = useCallback(
    (id: string) => {
      void navigate({
        to: '/audit-logs',
        search: searchWithPatch({ detail: id }),
      })
    },
    [navigate]
  )

  const closeDetail = useCallback(() => {
    void navigate({
      to: '/audit-logs',
      search: searchWithPatch({ detail: undefined }),
      replace: true,
    })
  }, [navigate])

  const clearFilters = useCallback(() => {
    void navigate({
      to: '/audit-logs',
      search: searchWithPatch(clearFiltersPatch()),
      replace: true,
    })
  }, [navigate])

  const resetRange = useCallback(() => {
    handleListChange(presetWindowPatch('30d'))
  }, [handleListChange])

  const openUser = useCallback(
    (userId: string) => {
      void navigate({ to: '/users', search: { user: userId } })
    },
    [navigate]
  )

  const openRole = useCallback(
    (roleId: string) => {
      void navigate({ to: '/roles/$roleId', params: { roleId } })
    },
    [navigate]
  )

  // An out-of-range page (deep link or a shrinking filter) returns to the
  // last real page once the fresh result arrives.
  useEffect(() => {
    if (!listQuery.data || listQuery.isPlaceholderData) return

    const pageCount = Math.max(1, Math.ceil(listQuery.data.total / pageSize))
    const currentPage = search.page ?? 1
    if (currentPage <= pageCount) return

    void navigate({
      to: '/audit-logs',
      search: searchWithPatch({
        page: pageCount > 1 ? pageCount : undefined,
        detail: undefined,
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
          <div className='grid gap-1'>
            <h2 className='text-2xl font-bold tracking-tight'>{t('title')}</h2>
            <p className='text-muted-foreground'>{t('description')}</p>
            <p className='flex items-center gap-1 text-xs text-muted-foreground'>
              <Info className='size-3.5 shrink-0' />
              {t('accessNotice')}
            </p>
          </div>
          <RetentionPolicy />
        </div>

        <AuditLogsTable
          data={listQuery.data}
          isPending={listQuery.isPending}
          isFetching={listQuery.isFetching}
          isError={listQuery.isError}
          error={listQuery.error}
          search={search}
          auditWindow={auditWindow}
          canReadUsers={canReadUsers}
          canReadRoles={canReadRoles}
          onListChange={handleListChange}
          onPaginationChange={handlePaginationChange}
          onSortingChange={handleSortingChange}
          onOpenDetail={openDetail}
          onOpenUser={openUser}
          onOpenRole={openRole}
          onRefresh={() => void listQuery.refetch()}
          onRetry={() => void listQuery.refetch()}
          onClearFilters={clearFilters}
          onResetRange={resetRange}
        />
      </Main>

      <AuditLogDetailSheet
        detailId={search.detail}
        onClose={closeDetail}
        onFilter={handleListChange}
        canReadUsers={canReadUsers}
        canReadRoles={canReadRoles}
        onOpenUser={openUser}
        onOpenRole={openRole}
      />
    </>
  )
}

function RetentionPolicy() {
  const { t } = useTranslation('audit')

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button type='button' variant='outline' size='sm'>
          <Info className='size-4' />
          {t('retention.button')}
        </Button>
      </PopoverTrigger>
      <PopoverContent align='end' className='grid w-80 gap-2 text-sm'>
        <p className='font-medium'>{t('retention.title')}</p>
        <p>{t('retention.infoWarning')}</p>
        <p>{t('retention.errorCritical')}</p>
        <p>{t('retention.maxRange')}</p>
        <p>{t('retention.purge')}</p>
        <p className='text-xs text-muted-foreground'>
          {t('retention.olderHint')}
        </p>
      </PopoverContent>
    </Popover>
  )
}

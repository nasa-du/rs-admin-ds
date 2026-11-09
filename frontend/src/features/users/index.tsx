import { useCallback, useEffect, useMemo, useState } from 'react'
import { keepPreviousData } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
import type { PaginationState, SortingState } from '@tanstack/react-table'
import { Plus } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { UserDetail, UserSummary } from '@/api/generated/model'
import { useListUsers } from '@/api/generated/rs-admin'
import { hasPermission } from '@/lib/permissions'
import { Button } from '@/components/ui/button'
import { ConfigDrawer } from '@/components/config-drawer'
import { Header } from '@/components/layout/header'
import { Main } from '@/components/layout/main'
import { ProfileDropdown } from '@/components/profile-dropdown'
import { Search } from '@/components/search'
import { ThemeSwitch } from '@/components/theme-switch'
import { Can } from '@/features/auth/can'
import { useAuth } from '@/features/auth/use-auth'
import { AssignRolesDialog } from './components/assign-roles-dialog'
import { UserCreateDialog } from './components/user-create-dialog'
import { UserDetailSheet } from './components/user-detail-sheet'
import { UserEditDialog } from './components/user-edit-dialog'
import { UsersTable } from './components/users-table'
import { useRoleOptions } from './use-role-options'
import {
  USERS_PAGE_SIZE_DEFAULT,
  clearFiltersPatch,
  listFiltersPatch,
  paginationPatch,
  sortingPatch,
  toListUsersParams,
  type UsersSearch,
} from './users-search'

/**
 * Users page (spec §9.8). URL search is the single source of truth for the
 * list query and the detail sheet; the server remains authoritative for the
 * data and for every permission decision.
 */
export function Users() {
  const { t } = useTranslation('users')
  const search = useSearch({ from: '/_authenticated/users/' })
  const navigate = useNavigate()
  const currentUser = useAuth()

  const pageSize = search.page_size ?? USERS_PAGE_SIZE_DEFAULT
  const canReadRoles = hasPermission(currentUser, 'roles.read')
  const canAssignRoles =
    hasPermission(currentUser, 'users.assign_role') && canReadRoles
  const canUpdate = hasPermission(currentUser, 'users.update')

  const [createOpen, setCreateOpen] = useState(false)
  const [editingUser, setEditingUser] = useState<UserSummary | null>(null)
  const [assigningRolesUser, setAssigningRolesUser] =
    useState<UserSummary | null>(null)

  const params = useMemo(() => toListUsersParams(search), [search])
  const listQuery = useListUsers(params, {
    query: { placeholderData: keepPreviousData },
  })
  const roleOptionsQuery = useRoleOptions(canReadRoles)

  const handleListChange = useCallback(
    (patch: Partial<UsersSearch>) => {
      void navigate({
        to: '/users',
        search: (prev) => ({ ...prev, ...listFiltersPatch(patch) }),
        replace: true,
      })
    },
    [navigate]
  )

  const handlePaginationChange = useCallback(
    (next: PaginationState) => {
      void navigate({
        to: '/users',
        search: (prev) => ({ ...prev, ...paginationPatch(next, pageSize) }),
        replace: true,
      })
    },
    [navigate, pageSize]
  )

  const handleSortingChange = useCallback(
    (next: SortingState) => {
      void navigate({
        to: '/users',
        search: (prev) => ({ ...prev, ...sortingPatch(next) }),
        replace: true,
      })
    },
    [navigate]
  )

  const openUser = useCallback(
    (id: string) => {
      void navigate({
        to: '/users',
        search: (prev) => ({ ...prev, user: id }),
      })
    },
    [navigate]
  )

  const closeUser = useCallback(() => {
    void navigate({
      to: '/users',
      search: (prev) => ({ ...prev, user: undefined }),
    })
  }, [navigate])

  const clearFilters = useCallback(() => {
    void navigate({
      to: '/users',
      search: (prev) => ({ ...prev, ...clearFiltersPatch() }),
      replace: true,
    })
  }, [navigate])

  // An out-of-range page (deep link or a shrinking filter) returns to the
  // last real page once the fresh result arrives (spec §9.8).
  useEffect(() => {
    if (!listQuery.data || listQuery.isPlaceholderData) return

    const pageCount = Math.max(1, Math.ceil(listQuery.data.total / pageSize))
    const currentPage = search.page ?? 1
    if (currentPage <= pageCount) return

    void navigate({
      to: '/users',
      search: (prev) => ({
        ...prev,
        page: pageCount > 1 ? pageCount : undefined,
        user: undefined,
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

  function handleEditFromSheet(detail: UserDetail) {
    setEditingUser(detail)
  }

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
          <Can permission='users.create'>
            <Button type='button' onClick={() => setCreateOpen(true)}>
              <Plus className='size-4' />
              {t('actions.create')}
            </Button>
          </Can>
        </div>

        <UsersTable
          data={listQuery.data}
          isPending={listQuery.isPending}
          isFetching={listQuery.isFetching}
          isError={listQuery.isError}
          error={listQuery.error}
          search={search}
          currentUser={currentUser}
          roleOptions={roleOptionsQuery.data ?? []}
          rolesLoading={roleOptionsQuery.isPending}
          rolesError={roleOptionsQuery.isError}
          onRetryRoles={() => void roleOptionsQuery.refetch()}
          onListChange={handleListChange}
          onPaginationChange={handlePaginationChange}
          onSortingChange={handleSortingChange}
          onOpenUser={openUser}
          onEditUser={setEditingUser}
          onAssignRoles={setAssigningRolesUser}
          onRefresh={() => void listQuery.refetch()}
          onRetry={() => void listQuery.refetch()}
          onClearFilters={clearFilters}
        />
      </Main>

      <UserDetailSheet
        userId={search.user}
        onClose={closeUser}
        onEdit={handleEditFromSheet}
        canEdit={canUpdate}
        currentUser={currentUser}
      />

      <UserCreateDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        canAssignRoles={canAssignRoles}
        currentUser={currentUser}
        roleOptions={roleOptionsQuery.data ?? []}
        rolesLoading={roleOptionsQuery.isPending && canAssignRoles}
        rolesError={roleOptionsQuery.isError}
        onRetryRoles={() => void roleOptionsQuery.refetch()}
      />

      {editingUser && (
        <UserEditDialog
          open
          user={editingUser}
          currentUser={currentUser}
          onOpenChange={(open) => {
            if (!open) setEditingUser(null)
          }}
        />
      )}

      {assigningRolesUser && (
        <AssignRolesDialog
          open
          user={assigningRolesUser}
          currentUser={currentUser}
          onOpenChange={(open) => {
            if (!open) setAssigningRolesUser(null)
          }}
        />
      )}
    </>
  )
}

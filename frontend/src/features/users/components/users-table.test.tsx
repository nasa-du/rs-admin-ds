import i18n from '@/i18n'
import { describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import type {
  UserListResponse,
  UserSelfResponse,
  UserSummary,
} from '@/api/generated/model'
import { ApiError } from '@/lib/api-error'
import { UsersTable } from './users-table'

const ADMIN: UserSelfResponse = {
  id: 'user-admin',
  username: 'admin',
  email: 'admin@example.com',
  display_name: 'Admin',
  status: 'active',
  must_change_password: false,
  is_super_admin: true,
  permissions: [],
  roles: [],
}

const READER: UserSelfResponse = {
  ...ADMIN,
  is_super_admin: false,
  permissions: ['users.read'],
}

function makeUser(overrides: Partial<UserSummary> = {}): UserSummary {
  return {
    id: 'user-alice',
    username: 'alice',
    email: 'alice@example.com',
    display_name: 'Alice',
    phone: null,
    status: 'active',
    roles: [{ id: 'role-user', name: 'user', is_builtin: true }],
    must_change_password: false,
    locked_until: null,
    last_login_at: null,
    created_at: '2026-01-02T03:04:05Z',
    updated_at: '2026-01-03T03:04:05Z',
    ...overrides,
  }
}

const PAGE: UserListResponse = {
  items: [
    makeUser(),
    makeUser({
      id: 'user-bob',
      username: 'bob',
      display_name: 'Bob',
      email: 'bob@example.com',
      status: 'disabled',
      must_change_password: true,
    }),
  ],
  total: 2,
  page: 1,
  page_size: 20,
}

function renderTable(
  overrides: Partial<Parameters<typeof UsersTable>[0]> = {}
) {
  return render(
    <UsersTable
      data={PAGE}
      isPending={false}
      isFetching={false}
      isError={false}
      error={null}
      search={{}}
      currentUser={ADMIN}
      roleOptions={[]}
      rolesLoading={false}
      rolesError={false}
      onRetryRoles={vi.fn()}
      onListChange={vi.fn()}
      onPaginationChange={vi.fn()}
      onSortingChange={vi.fn()}
      onOpenUser={vi.fn()}
      onEditUser={vi.fn()}
      onRefresh={vi.fn()}
      onRetry={vi.fn()}
      onClearFilters={vi.fn()}
      {...overrides}
    />
  )
}

describe('UsersTable', () => {
  it('renders the server page and opens the detail sheet on row click', async () => {
    const onOpenUser = vi.fn()
    const screen = await renderTable({ onOpenUser })

    await expect
      .element(screen.getByText('alice', { exact: true }))
      .toBeInTheDocument()
    await expect
      .element(screen.getByText('bob@example.com'))
      .toBeInTheDocument()
    await expect
      .element(screen.getByText(i18n.t('users:status.disabled')))
      .toBeInTheDocument()
    await expect
      .element(screen.getByText(i18n.t('users:status.mustChangePassword')))
      .toBeInTheDocument()

    await userEvent.click(screen.getByText('alice', { exact: true }))

    expect(onOpenUser).toHaveBeenCalledWith('user-alice')
  })

  it('opens the edit action without triggering the row navigation', async () => {
    const onEditUser = vi.fn()
    const onOpenUser = vi.fn()
    const screen = await renderTable({ onEditUser, onOpenUser })

    await userEvent.click(
      screen
        .getByRole('button', {
          name: i18n.t('users:actions.more'),
          exact: true,
        })
        .first()
    )
    await userEvent.click(
      screen.getByRole('menuitem', { name: i18n.t('users:actions.edit') })
    )

    expect(onEditUser).toHaveBeenCalledWith(PAGE.items[0])
    expect(onOpenUser).not.toHaveBeenCalled()
  })

  it('disables the edit action on the current user', async () => {
    const onEditUser = vi.fn()
    const screen = await renderTable({
      currentUser: { ...ADMIN, id: PAGE.items[0].id },
      onEditUser,
    })

    await userEvent.click(
      screen
        .getByRole('button', {
          name: i18n.t('users:actions.more'),
          exact: true,
        })
        .first()
    )

    const editItem = screen.getByRole('menuitem', {
      name: i18n.t('users:actions.edit'),
    })
    await expect.element(editItem).toHaveAttribute('aria-disabled', 'true')

    await userEvent.hover(editItem)
    await expect
      .element(screen.getByText(i18n.t('users:edit.self')))
      .toBeInTheDocument()

    expect(onEditUser).not.toHaveBeenCalled()
  })

  it('writes the selected sort into the URL state', async () => {
    const onListChange = vi.fn()
    const screen = await renderTable({ onListChange })

    await userEvent.click(
      screen.getByRole('combobox', { name: i18n.t('users:filters.sort') })
    )
    await userEvent.click(
      screen.getByRole('option', { name: i18n.t('users:sort.usernameDesc') })
    )

    expect(onListChange).toHaveBeenCalledWith({ sort: '-username' })
  })

  it('hides the actions column without users.update', async () => {
    const screen = await renderTable({ currentUser: READER })

    await expect
      .element(screen.getByText(i18n.t('users:columns.actions')))
      .not.toBeInTheDocument()
  })

  it('shows a skeleton while the first page loads', async () => {
    const screen = await renderTable({
      data: undefined,
      isPending: true,
    })

    await expect
      .element(screen.getByText(i18n.t('users:states.loading')))
      .toBeInTheDocument()
  })

  it('distinguishes an empty account list from an empty search', async () => {
    const empty: UserListResponse = {
      items: [],
      total: 0,
      page: 1,
      page_size: 20,
    }
    const onClearFilters = vi.fn()
    const screen = await renderTable({ data: empty, onClearFilters })

    await expect
      .element(screen.getByText(i18n.t('users:states.noUsers.title')))
      .toBeInTheDocument()

    await screen.rerender(
      <UsersTable
        data={empty}
        isPending={false}
        isFetching={false}
        isError={false}
        error={null}
        search={{ q: 'missing' }}
        currentUser={ADMIN}
        roleOptions={[]}
        rolesLoading={false}
        rolesError={false}
        onRetryRoles={vi.fn()}
        onListChange={vi.fn()}
        onPaginationChange={vi.fn()}
        onSortingChange={vi.fn()}
        onOpenUser={vi.fn()}
        onEditUser={vi.fn()}
        onRefresh={vi.fn()}
        onRetry={vi.fn()}
        onClearFilters={onClearFilters}
      />
    )

    await expect
      .element(screen.getByText(i18n.t('users:states.noMatches.title')))
      .toBeInTheDocument()

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('users:states.clearFilters') })
    )
    expect(onClearFilters).toHaveBeenCalledTimes(1)
  })

  it('shows a retryable error instead of silently clearing the table', async () => {
    const onRetry = vi.fn()
    const screen = await renderTable({
      data: undefined,
      isError: true,
      error: new ApiError({ code: 'service_unavailable', status: 503 }),
      onRetry,
    })

    await expect
      .element(screen.getByText(i18n.t('errors:common.service_unavailable')))
      .toBeInTheDocument()

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('common:error.retry') })
    )
    expect(onRetry).toHaveBeenCalledTimes(1)
  })
})

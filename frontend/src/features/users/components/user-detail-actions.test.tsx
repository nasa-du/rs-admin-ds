import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import type { UserDetail, UserSelfResponse } from '@/api/generated/model'
import {
  getGetUserQueryKey,
  getListUsersQueryKey,
} from '@/api/generated/rs-admin'
import { ApiError } from '@/lib/api-error'
import { UserDetailActions } from './user-detail-actions'

const mocks = vi.hoisted(() => ({
  setStatus: vi.fn(),
  resetPassword: vi.fn(),
  unlock: vi.fn(),
  setRoles: vi.fn(),
  revokeSessions: vi.fn(),
  listRoles: vi.fn(),
  runWithReauth: vi.fn(),
}))

vi.mock('@/api/generated/rs-admin', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/api/generated/rs-admin')>()
  return {
    ...actual,
    useSetUserStatus: () => ({
      mutateAsync: mocks.setStatus,
      reset: vi.fn(),
      isPending: false,
    }),
    useResetUserPassword: () => ({
      mutateAsync: mocks.resetPassword,
      reset: vi.fn(),
      isPending: false,
    }),
    useUnlockUser: () => ({
      mutateAsync: mocks.unlock,
      reset: vi.fn(),
      isPending: false,
    }),
    useSetUserRoles: () => ({
      mutateAsync: mocks.setRoles,
      reset: vi.fn(),
      isPending: false,
    }),
    useRevokeUserSessions: () => ({
      mutateAsync: mocks.revokeSessions,
      reset: vi.fn(),
      isPending: false,
    }),
    listRoles: (...args: unknown[]) => mocks.listRoles(...args),
  }
})

vi.mock('@/features/auth/reauth/reauth', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/features/auth/reauth/reauth')>()
  return {
    ...actual,
    runWithReauth: (fn: () => Promise<unknown>) => mocks.runWithReauth(fn),
  }
})

const DETAIL: UserDetail = {
  id: 'user-alice',
  username: 'alice',
  email: 'alice@example.com',
  display_name: 'Alice',
  phone: null,
  status: 'active',
  roles: [
    { id: 'role-user', name: 'user', is_builtin: true },
    { id: 'role-support', name: 'support', is_builtin: false },
  ],
  must_change_password: false,
  locked_until: null,
  last_login_at: null,
  temporary_password_expires_at: null,
  created_at: '2026-01-02T03:04:05Z',
  updated_at: '2026-01-03T03:04:05Z',
}

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

const ROLES = [
  {
    id: 'role-user',
    name: 'user',
    description: null,
    is_builtin: true,
    permission_keys: [],
    assignable: false,
    manageable: false,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
  },
  {
    id: 'role-support',
    name: 'support',
    description: null,
    is_builtin: false,
    permission_keys: ['users.read'],
    assignable: true,
    manageable: false,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
  },
]

async function renderActions({
  user = DETAIL,
  currentUser = ADMIN,
  canEdit = true,
  onEdit = vi.fn(),
}: {
  user?: UserDetail
  currentUser?: UserSelfResponse | null
  canEdit?: boolean
  onEdit?: (user: UserDetail) => void
} = {}) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  })
  const invalidate = vi.spyOn(queryClient, 'invalidateQueries')

  const screen = await render(
    <QueryClientProvider client={queryClient}>
      <UserDetailActions
        user={user}
        currentUser={currentUser}
        canEdit={canEdit}
        onEdit={onEdit}
      />
    </QueryClientProvider>
  )

  return { screen, queryClient, invalidate, onEdit }
}

const ACTION_NAMES = {
  assignRoles: 'users:manage.assignRoles.action',
  resetPassword: 'users:manage.resetPassword.action',
  unlock: 'users:manage.unlock.action',
  disable: 'users:manage.status.disableAction',
  enable: 'users:manage.status.enableAction',
  disableConfirm: 'users:manage.status.disableConfirm',
  revokeSessions: 'users:manage.revokeSessions.action',
  revokeConfirm: 'users:manage.revokeSessions.confirm',
} as const

function actionName(key: keyof typeof ACTION_NAMES) {
  return i18n.t(ACTION_NAMES[key])
}

describe('UserDetailActions', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.runWithReauth.mockImplementation((fn: () => Promise<unknown>) => fn())
    mocks.setStatus.mockResolvedValue(DETAIL)
    mocks.resetPassword.mockResolvedValue({
      user: DETAIL,
      temporary_password: 'reset-temp-password',
      temporary_password_expires_at: '2026-11-12T00:00:00Z',
    })
    mocks.unlock.mockResolvedValue(DETAIL)
    mocks.setRoles.mockResolvedValue(DETAIL)
    mocks.revokeSessions.mockResolvedValue(undefined)
    mocks.listRoles.mockResolvedValue({
      items: ROLES,
      total: ROLES.length,
      page: 1,
      page_size: 200,
    })
  })

  it('hides every action when the current user has no matching permission', async () => {
    const reader: UserSelfResponse = {
      ...ADMIN,
      id: 'user-reader',
      is_super_admin: false,
      permissions: ['users.read'],
    }
    const { screen } = await renderActions({
      currentUser: reader,
      canEdit: false,
    })

    expect(screen.container.textContent).toBe('')
  })

  it('renders only the actions the current user is allowed to perform', async () => {
    const { screen } = await renderActions({
      currentUser: {
        ...ADMIN,
        is_super_admin: false,
        permissions: ['users.read', 'users.set_status'],
      },
      canEdit: false,
    })

    await expect
      .element(screen.getByRole('button', { name: actionName('disable') }))
      .toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: actionName('assignRoles') })
    ).not.toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: actionName('resetPassword') })
    ).not.toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: actionName('revokeSessions') })
    ).not.toBeInTheDocument()
  })

  it('requires roles.read in addition to users.assign_role', async () => {
    const { screen } = await renderActions({
      currentUser: {
        ...ADMIN,
        is_super_admin: false,
        permissions: ['users.read', 'users.assign_role'],
      },
      canEdit: false,
    })

    expect(
      screen.getByRole('button', { name: actionName('assignRoles') })
    ).not.toBeInTheDocument()
  })

  it('disables every management action on the current user', async () => {
    const { screen } = await renderActions({
      currentUser: { ...ADMIN, id: DETAIL.id },
    })

    await expect
      .element(screen.getByRole('button', { name: actionName('assignRoles') }))
      .toBeDisabled()
    await expect
      .element(
        screen.getByRole('button', { name: actionName('resetPassword') })
      )
      .toBeDisabled()
    await expect
      .element(screen.getByRole('button', { name: actionName('disable') }))
      .toBeDisabled()
    await expect
      .element(
        screen.getByRole('button', { name: actionName('revokeSessions') })
      )
      .toBeDisabled()
    await expect
      .element(
        screen.getByRole('button', { name: i18n.t('users:detail.edit') })
      )
      .toBeDisabled()
  })

  it('only shows unlock while the account is currently locked', async () => {
    const future = new Date(Date.now() + 60_000).toISOString()
    const locked = await renderActions({
      user: { ...DETAIL, locked_until: future },
    })

    await expect
      .element(
        locked.screen.getByRole('button', { name: actionName('unlock') })
      )
      .toBeInTheDocument()

    await locked.screen.unmount()

    const past = new Date(Date.now() - 60_000).toISOString()
    const expired = await renderActions({
      user: { ...DETAIL, locked_until: past },
    })
    expect(
      expired.screen.getByRole('button', { name: actionName('unlock') })
    ).not.toBeInTheDocument()
  })

  it('unlocks without reauthentication and refreshes the detail and list', async () => {
    const future = new Date(Date.now() + 60_000).toISOString()
    const { screen, invalidate } = await renderActions({
      user: { ...DETAIL, locked_until: future },
    })

    await userEvent.click(
      screen.getByRole('button', { name: actionName('unlock') })
    )

    await vi.waitFor(() => {
      expect(mocks.unlock).toHaveBeenCalledWith({ id: DETAIL.id })
    })
    expect(mocks.runWithReauth).not.toHaveBeenCalled()
    expect(invalidate).toHaveBeenCalledWith(
      expect.objectContaining({ queryKey: getGetUserQueryKey(DETAIL.id) })
    )
    expect(invalidate).toHaveBeenCalledWith(
      expect.objectContaining({ queryKey: getListUsersQueryKey() })
    )
  })

  it('confirms before disabling and keeps the dialog open on last_super_admin', async () => {
    mocks.setStatus.mockRejectedValueOnce(
      new ApiError({ code: 'last_super_admin', status: 409 })
    )
    const { screen } = await renderActions()

    await userEvent.click(
      screen.getByRole('button', { name: actionName('disable') })
    )
    await expect.element(screen.getByRole('alertdialog')).toBeInTheDocument()
    await expect
      .element(
        screen.getByText(
          i18n.t('users:manage.status.disableTitle', {
            username: DETAIL.username,
          })
        )
      )
      .toBeInTheDocument()

    await userEvent.click(
      screen.getByRole('button', { name: actionName('disableConfirm') })
    )

    await vi.waitFor(() => {
      expect(mocks.setStatus).toHaveBeenCalledWith({
        id: DETAIL.id,
        data: { status: 'disabled' },
      })
    })
    await expect
      .element(screen.getByText(i18n.t('errors:user.last_super_admin')))
      .toBeInTheDocument()
    await expect.element(screen.getByRole('alertdialog')).toBeInTheDocument()
  })

  it('enables a disabled user without a confirmation step', async () => {
    const { screen } = await renderActions({
      user: { ...DETAIL, status: 'disabled' },
    })

    await userEvent.click(
      screen.getByRole('button', { name: actionName('enable') })
    )

    await vi.waitFor(() => {
      expect(mocks.setStatus).toHaveBeenCalledWith({
        id: DETAIL.id,
        data: { status: 'active' },
      })
    })
    expect(screen.getByRole('alertdialog')).not.toBeInTheDocument()
  })

  it('confirms before revoking every session', async () => {
    const { screen, invalidate } = await renderActions()

    await userEvent.click(
      screen.getByRole('button', { name: actionName('revokeSessions') })
    )
    await expect
      .element(
        screen.getByText(
          i18n.t('users:manage.revokeSessions.title', {
            username: DETAIL.username,
          })
        )
      )
      .toBeInTheDocument()

    await userEvent.click(
      screen.getByRole('button', {
        name: actionName('revokeConfirm'),
      })
    )

    await vi.waitFor(() => {
      expect(mocks.revokeSessions).toHaveBeenCalledWith({ id: DETAIL.id })
    })
    expect(mocks.runWithReauth).toHaveBeenCalledTimes(1)
    expect(invalidate).toHaveBeenCalledWith(
      expect.objectContaining({ queryKey: getGetUserQueryKey(DETAIL.id) })
    )
  })

  it('opens the reset-password and role-assignment dialogs', async () => {
    const { screen } = await renderActions()

    await userEvent.click(
      screen.getByRole('button', { name: actionName('resetPassword') })
    )
    await expect
      .element(
        screen.getByRole('dialog', {
          name: i18n.t('users:manage.resetPassword.title'),
        })
      )
      .toBeInTheDocument()

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('common:actions.cancel') })
    )

    await userEvent.click(
      screen.getByRole('button', { name: actionName('assignRoles') })
    )
    await expect
      .element(
        screen.getByRole('dialog', {
          name: i18n.t('users:manage.assignRoles.title'),
        })
      )
      .toBeInTheDocument()
  })
})

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import type {
  RoleSummary,
  UserDetail,
  UserSelfResponse,
} from '@/api/generated/model'
import {
  getGetUserQueryKey,
  getListUsersQueryKey,
} from '@/api/generated/rs-admin'
import { ApiError } from '@/lib/api-error'
import { AssignRolesDialog } from './assign-roles-dialog'

const mocks = vi.hoisted(() => ({
  setRoles: vi.fn(),
  listRoles: vi.fn(),
  runWithReauth: vi.fn(),
}))

vi.mock('@/api/generated/rs-admin', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/api/generated/rs-admin')>()
  return {
    ...actual,
    useSetUserRoles: () => ({
      mutateAsync: mocks.setRoles,
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

const USER: Pick<UserDetail, 'id' | 'username' | 'roles'> = {
  id: 'user-alice',
  username: 'alice',
  roles: [
    { id: 'role-user', name: 'user', is_builtin: true },
    { id: 'role-support', name: 'support', is_builtin: false },
  ],
}

const ROLES: RoleSummary[] = [
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
  {
    id: 'role-admin',
    name: 'admin',
    description: null,
    is_builtin: true,
    permission_keys: [],
    assignable: false,
    manageable: false,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
  },
  {
    id: 'role-auditor',
    name: 'auditor',
    description: null,
    is_builtin: false,
    permission_keys: ['audit.read'],
    assignable: false,
    manageable: false,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
  },
  {
    id: 'role-helper',
    name: 'helper',
    description: null,
    is_builtin: false,
    permission_keys: [],
    assignable: true,
    manageable: false,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
  },
]

const ADMIN: UserSelfResponse = {
  id: 'user-admin',
  username: 'admin',
  email: 'admin@example.com',
  display_name: 'Admin',
  status: 'active',
  must_change_password: false,
  is_super_admin: false,
  permissions: ['users.assign_role', 'roles.read'],
  roles: [],
}

async function renderDialog({
  currentUser = ADMIN,
  onOpenChange = vi.fn(),
}: {
  currentUser?: UserSelfResponse
  onOpenChange?: (open: boolean) => void
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
      <AssignRolesDialog
        open
        onOpenChange={onOpenChange}
        user={USER}
        currentUser={currentUser}
      />
    </QueryClientProvider>
  )

  return { screen, queryClient, invalidate, onOpenChange }
}

describe('AssignRolesDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.runWithReauth.mockImplementation((fn: () => Promise<unknown>) => fn())
    mocks.setRoles.mockResolvedValue(USER)
    mocks.listRoles.mockResolvedValue({
      items: ROLES,
      total: ROLES.length,
      page: 1,
      page_size: 200,
    })
  })

  it('preselects the current extra roles and always keeps the baseline checked', async () => {
    const { screen } = await renderDialog()

    const baseline = screen.getByRole('checkbox', { name: /user/i })
    await expect.element(baseline).toBeChecked()
    await expect.element(baseline).toBeDisabled()
    await expect
      .element(screen.getByRole('checkbox', { name: /support/ }))
      .toBeChecked()
  })

  it('submits the complete extra role set without the baseline', async () => {
    const { screen, invalidate, onOpenChange } = await renderDialog()

    await waitForRoles(screen)
    await userEvent.click(screen.getByRole('checkbox', { name: /support/ }))
    await userEvent.click(screen.getByRole('checkbox', { name: /helper/ }))

    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('users:manage.assignRoles.submit'),
      })
    )

    await vi.waitFor(() => {
      expect(mocks.setRoles).toHaveBeenCalledWith({
        id: USER.id,
        data: { role_ids: ['role-helper'] },
      })
    })
    expect(mocks.runWithReauth).toHaveBeenCalledTimes(1)
    expect(invalidate).toHaveBeenCalledWith(
      expect.objectContaining({ queryKey: getGetUserQueryKey(USER.id) })
    )
    expect(invalidate).toHaveBeenCalledWith(
      expect.objectContaining({ queryKey: getListUsersQueryKey() })
    )
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })

  it('keeps privileged and non-grantable roles visible but disabled with a reason', async () => {
    const { screen } = await renderDialog()

    await waitForRoles(screen)

    await expect
      .element(screen.getByRole('checkbox', { name: /admin/ }))
      .toBeDisabled()
    await expect
      .element(screen.getByText(i18n.t('users:create.rolePrivilegedOnly')))
      .toBeInTheDocument()
    await expect
      .element(screen.getByRole('checkbox', { name: /auditor/ }))
      .toBeDisabled()
    await expect
      .element(screen.getByText(i18n.t('users:create.roleNotAssignable')))
      .toBeInTheDocument()
  })

  it('lets a super admin grant a privileged role and warns about it', async () => {
    mocks.listRoles.mockResolvedValue({
      items: ROLES.map((role) =>
        role.name === 'admin' ? { ...role, assignable: true } : role
      ),
      total: ROLES.length,
      page: 1,
      page_size: 200,
    })
    const { screen } = await renderDialog({
      currentUser: { ...ADMIN, is_super_admin: true, permissions: [] },
    })

    await waitForRoles(screen)
    await userEvent.click(screen.getByRole('checkbox', { name: /admin/ }))

    await expect
      .element(
        screen.getByText(i18n.t('users:manage.assignRoles.privilegedChange'))
      )
      .toBeInTheDocument()
  })

  it('maps a /role_ids field error into the role area', async () => {
    mocks.setRoles.mockRejectedValueOnce(
      new ApiError({
        code: 'validation_failed',
        status: 422,
        errors: [{ pointer: '/role_ids/0', code: 'invalid' }],
      })
    )
    const { screen, onOpenChange } = await renderDialog()

    await waitForRoles(screen)
    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('users:manage.assignRoles.submit'),
      })
    )

    await expect
      .element(screen.getByText(i18n.t('validation:invalid')))
      .toBeInTheDocument()
    expect(onOpenChange).not.toHaveBeenCalled()
  })

  it('shows permission_subset_violation and last_super_admin inline', async () => {
    mocks.setRoles.mockRejectedValueOnce(
      new ApiError({ code: 'permission_subset_violation', status: 403 })
    )
    const { screen, onOpenChange } = await renderDialog()

    await waitForRoles(screen)
    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('users:manage.assignRoles.submit'),
      })
    )

    await expect
      .element(
        screen.getByText(i18n.t('errors:role.permission_subset_violation'))
      )
      .toBeInTheDocument()
    expect(onOpenChange).not.toHaveBeenCalled()

    mocks.setRoles.mockRejectedValueOnce(
      new ApiError({ code: 'last_super_admin', status: 409 })
    )
    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('users:manage.assignRoles.submit'),
      })
    )
    await expect
      .element(screen.getByText(i18n.t('errors:user.last_super_admin')))
      .toBeInTheDocument()
  })
})

async function waitForRoles(
  screen: Awaited<ReturnType<typeof renderDialog>>['screen']
) {
  await expect
    .element(screen.getByRole('checkbox', { name: /support/ }))
    .toBeInTheDocument()
}

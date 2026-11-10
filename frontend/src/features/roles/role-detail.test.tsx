import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import type {
  PermissionListResponse,
  RoleDetail as RoleDetailModel,
  UserSelfResponse,
} from '@/api/generated/model'
import { getGetRoleQueryKey } from '@/api/generated/rs-admin'
import { ApiError } from '@/lib/api-error'
import { RoleDetail } from './role-detail'

const mocks = vi.hoisted(() => ({
  useGetRole: vi.fn(),
  useListPermissions: vi.fn(),
  useUpdateRoleMutation: vi.fn(),
  useSetRolePermissionsMutation: vi.fn(),
  updateRole: vi.fn(),
  setRolePermissions: vi.fn(),
  currentUser: null as UserSelfResponse | null,
  runWithReauth: vi.fn(),
  navigate: vi.fn(),
  useBlocker: vi.fn(),
  toastSuccess: vi.fn(),
  queryClient: null as QueryClient | null,
}))

vi.mock('@/api/generated/rs-admin', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/api/generated/rs-admin')>()
  return {
    ...actual,
    useGetRole: (...args: unknown[]) => mocks.useGetRole(...args),
    useListPermissions: (...args: unknown[]) =>
      mocks.useListPermissions(...args),
    useUpdateRole: (...args: unknown[]) =>
      mocks.useUpdateRoleMutation(...args) ?? {
        mutateAsync: mocks.updateRole,
        isPending: false,
      },
    useSetRolePermissions: (...args: unknown[]) =>
      mocks.useSetRolePermissionsMutation(...args) ?? {
        mutateAsync: mocks.setRolePermissions,
        isPending: false,
      },
    useDeleteRole: () => ({ mutateAsync: vi.fn(), isPending: false }),
  }
})

vi.mock('@/features/auth/use-auth', () => ({
  useAuth: () => mocks.currentUser,
  usePermission: () => false,
}))

vi.mock('@/features/auth/reauth/reauth', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/features/auth/reauth/reauth')>()
  return {
    ...actual,
    runWithReauth: (fn: () => Promise<unknown>) => mocks.runWithReauth(fn),
  }
})

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>()
  return {
    ...actual,
    useParams: () => ({ roleId: 'role-support' }),
    useNavigate: () => mocks.navigate,
    useBlocker: (...args: unknown[]) => mocks.useBlocker(...args),
  }
})

vi.mock('sonner', () => ({
  toast: {
    success: (...args: unknown[]) => mocks.toastSuccess(...args),
  },
}))

vi.mock('@/components/layout/header', () => ({
  Header: ({ children }: { children: React.ReactNode }) => (
    <header>{children}</header>
  ),
}))
vi.mock('@/components/search', () => ({ Search: () => null }))
vi.mock('@/components/theme-switch', () => ({ ThemeSwitch: () => null }))
vi.mock('@/components/config-drawer', () => ({ ConfigDrawer: () => null }))
vi.mock('@/components/profile-dropdown', () => ({
  ProfileDropdown: () => null,
}))

const CATALOG: PermissionListResponse = {
  groups: [
    {
      key: 'users',
      display_name_zh: '用户',
      display_name_en: 'Users',
      permissions: [
        {
          key: 'users.read',
          display_name_zh: '查看用户',
          display_name_en: 'View users',
          grantable: true,
        },
        {
          key: 'users.create',
          display_name_zh: '创建用户',
          display_name_en: 'Create users',
          grantable: true,
        },
        {
          key: 'users.update',
          display_name_zh: '编辑用户',
          display_name_en: 'Edit users',
          grantable: false,
        },
      ],
    },
    {
      key: 'roles',
      display_name_zh: '角色',
      display_name_en: 'Roles',
      permissions: [
        {
          key: 'roles.read',
          display_name_zh: '查看角色',
          display_name_en: 'View roles',
          grantable: true,
        },
      ],
    },
  ],
}

const CUSTOM_ROLE: RoleDetailModel = {
  id: 'role-support',
  name: 'support',
  description: 'Support staff',
  is_builtin: false,
  permission_keys: ['users.read'],
  assignable: true,
  manageable: true,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-02T00:00:00Z',
}

const SUPER_ADMIN_ROLE: RoleDetailModel = {
  id: 'role-super-admin',
  name: 'super_admin',
  description: null,
  is_builtin: true,
  permission_keys: ['users.read', 'users.create', 'users.update', 'roles.read'],
  assignable: true,
  manageable: false,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
}

const UNMANAGEABLE_ROLE: RoleDetailModel = {
  ...CUSTOM_ROLE,
  id: 'role-other',
  name: 'other-team',
  manageable: false,
}

function makeUser(permissions: string[]): UserSelfResponse {
  return {
    id: 'user-admin',
    username: 'admin',
    email: 'admin@example.com',
    display_name: 'Admin',
    status: 'active',
    must_change_password: false,
    is_super_admin: false,
    permissions,
    roles: [],
  }
}

const MANAGER = makeUser([
  'roles.read',
  'roles.update',
  'roles.delete',
  'users.read',
])

function renderPage(role: RoleDetailModel = CUSTOM_ROLE) {
  mocks.useGetRole.mockReturnValue({
    data: role,
    isPending: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  })

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  mocks.queryClient = queryClient

  return render(
    <QueryClientProvider client={queryClient}>
      <RoleDetail />
    </QueryClientProvider>
  )
}

describe('RoleDetail', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.currentUser = MANAGER
    mocks.useListPermissions.mockReturnValue({
      data: CATALOG,
      isPending: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    })
    mocks.useUpdateRoleMutation.mockReturnValue(undefined)
    mocks.useSetRolePermissionsMutation.mockReturnValue(undefined)
    mocks.runWithReauth.mockImplementation((fn: () => Promise<unknown>) => fn())
    mocks.useBlocker.mockReturnValue({
      status: 'idle',
      proceed: undefined,
      reset: undefined,
    })
    mocks.updateRole.mockResolvedValue({ ...CUSTOM_ROLE, name: 'support-2' })
    mocks.setRolePermissions.mockResolvedValue({
      ...CUSTOM_ROLE,
      permission_keys: ['users.read', 'users.create'],
    })
  })

  it('renders a built-in role as a localized read-only matrix', async () => {
    const screen = await renderPage(SUPER_ADMIN_ROLE)

    await expect
      .element(screen.getByText(i18n.t('roles:builtin.super_admin')).first())
      .toBeInTheDocument()
    await expect.element(screen.getByText('super_admin')).toBeInTheDocument()
    await expect
      .element(screen.getByText(i18n.t('roles:detail.superAdminBanner')))
      .toBeInTheDocument()
    await expect
      .element(screen.getByRole('checkbox', { name: 'View users' }))
      .toBeDisabled()
    await expect
      .element(screen.getByRole('checkbox', { name: 'Edit users' }))
      .toBeChecked()
    await expect
      .element(
        screen.getByRole('button', { name: i18n.t('roles:detail.save') })
      )
      .not.toBeInTheDocument()
    await expect
      .element(
        screen.getByRole('button', { name: i18n.t('roles:detail.delete') })
      )
      .not.toBeInTheDocument()
  })

  it('saves only dirty metadata with PATCH', async () => {
    const screen = await renderPage()

    await userEvent.fill(
      screen.getByRole('textbox', { name: i18n.t('roles:detail.name') }),
      'support-2'
    )
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('roles:detail.save') }).first()
    )

    expect(mocks.updateRole).toHaveBeenCalledWith({
      id: CUSTOM_ROLE.id,
      data: { name: 'support-2' },
    })
    expect(mocks.setRolePermissions).not.toHaveBeenCalled()
    expect(
      mocks.queryClient?.getQueryData(getGetRoleQueryKey(CUSTOM_ROLE.id))
    ).toMatchObject({ name: 'support-2' })
  })

  it('saves only dirty permissions with PUT', async () => {
    const screen = await renderPage()

    await userEvent.click(
      screen.getByRole('checkbox', { name: 'Create users' })
    )
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('roles:detail.save') }).first()
    )

    expect(mocks.setRolePermissions).toHaveBeenCalledWith({
      id: CUSTOM_ROLE.id,
      data: { permission_keys: ['users.read', 'users.create'] },
    })
    expect(mocks.updateRole).not.toHaveBeenCalled()
  })

  it('saves metadata first, then permissions, when both are dirty', async () => {
    const screen = await renderPage()

    await userEvent.fill(
      screen.getByRole('textbox', { name: i18n.t('roles:detail.name') }),
      'support-2'
    )
    await userEvent.click(
      screen.getByRole('checkbox', { name: 'Create users' })
    )
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('roles:detail.save') }).first()
    )

    expect(mocks.updateRole.mock.invocationCallOrder[0]).toBeLessThan(
      mocks.setRolePermissions.mock.invocationCallOrder[0]
    )
    expect(mocks.runWithReauth).toHaveBeenCalledTimes(2)
  })

  it('keeps permissions dirty after a partial failure and retries only the PUT', async () => {
    mocks.setRolePermissions.mockRejectedValue(
      new ApiError({ code: 'internal_error', status: 500 })
    )
    const screen = await renderPage()

    await userEvent.fill(
      screen.getByRole('textbox', { name: i18n.t('roles:detail.name') }),
      'support-2'
    )
    await userEvent.click(
      screen.getByRole('checkbox', { name: 'Create users' })
    )
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('roles:detail.save') }).first()
    )

    await expect
      .element(screen.getByText(i18n.t('roles:detail.partialFailure')))
      .toBeInTheDocument()

    const saveButton = screen
      .getByRole('button', { name: i18n.t('roles:detail.save') })
      .first()
    await expect.element(saveButton).toBeEnabled()

    await userEvent.click(saveButton)

    expect(mocks.updateRole).toHaveBeenCalledTimes(1)
    expect(mocks.setRolePermissions).toHaveBeenCalledTimes(2)
  })

  it('blocks saving an empty permission set', async () => {
    const screen = await renderPage()

    await userEvent.click(screen.getByRole('checkbox', { name: 'View users' }))
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('roles:detail.save') }).first()
    )

    await expect
      .element(screen.getByText(i18n.t('validation:selectAtLeastOneItem')))
      .toBeInTheDocument()
    expect(mocks.updateRole).not.toHaveBeenCalled()
    expect(mocks.setRolePermissions).not.toHaveBeenCalled()
  })

  it('links to the filtered user list when users.read is available', async () => {
    const screen = await renderPage()

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('roles:detail.viewUsers') })
    )

    expect(mocks.navigate).toHaveBeenCalledWith({
      to: '/users',
      search: { role_id: CUSTOM_ROLE.id },
    })
  })

  it('renders an unmanageable custom role read-only', async () => {
    const screen = await renderPage(UNMANAGEABLE_ROLE)

    await expect
      .element(screen.getByText(i18n.t('roles:detail.notManageableBanner')))
      .toBeInTheDocument()
    await expect
      .element(
        screen.getByRole('button', { name: i18n.t('roles:detail.save') })
      )
      .not.toBeInTheDocument()
    await expect
      .element(
        screen.getByRole('button', { name: i18n.t('roles:detail.delete') })
      )
      .toBeDisabled()
  })

  it('shows a not-found state for a deleted or unknown role', async () => {
    mocks.useGetRole.mockReturnValue({
      data: undefined,
      isPending: false,
      isError: true,
      error: new ApiError({ code: 'not_found', status: 404 }),
      refetch: vi.fn(),
    })
    const queryClient = new QueryClient({
      defaultOptions: {},
    })

    const screen = await render(
      <QueryClientProvider client={queryClient}>
        <RoleDetail />
      </QueryClientProvider>
    )

    await expect
      .element(screen.getByText(i18n.t('roles:states.notFound.title')))
      .toBeInTheDocument()
    await expect
      .element(
        screen.getByRole('button', { name: i18n.t('roles:detail.save') })
      )
      .not.toBeInTheDocument()
  })
})

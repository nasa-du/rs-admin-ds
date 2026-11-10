import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import type { PermissionListResponse, RoleDetail } from '@/api/generated/model'
import { ApiError } from '@/lib/api-error'
import { RoleCreate } from './role-create'

const mocks = vi.hoisted(() => ({
  useListPermissions: vi.fn(),
  createRole: vi.fn(),
  runWithReauth: vi.fn(),
  navigate: vi.fn(),
  useBlocker: vi.fn(),
  toastSuccess: vi.fn(),
}))

vi.mock('@/api/generated/rs-admin', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/api/generated/rs-admin')>()
  return {
    ...actual,
    useListPermissions: (...args: unknown[]) =>
      mocks.useListPermissions(...args),
    useCreateRole: () => ({
      mutateAsync: mocks.createRole,
      reset: vi.fn(),
      isPending: false,
    }),
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

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>()
  return {
    ...actual,
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
          key: 'users.update',
          display_name_zh: '编辑用户',
          display_name_en: 'Edit users',
          grantable: false,
        },
      ],
    },
  ],
}

const CREATED: RoleDetail = {
  id: 'role-support',
  name: 'support',
  description: 'Support staff',
  is_builtin: false,
  permission_keys: ['users.read'],
  assignable: true,
  manageable: true,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
}

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })

  return render(
    <QueryClientProvider client={queryClient}>
      <RoleCreate />
    </QueryClientProvider>
  )
}

async function fillNameAndPermission(
  screen: Awaited<ReturnType<typeof renderPage>>
) {
  await userEvent.fill(
    screen.getByRole('textbox', { name: i18n.t('roles:new.name') }),
    ' support '
  )
  await userEvent.click(screen.getByRole('checkbox', { name: 'View users' }))
}

describe('RoleCreate', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.useListPermissions.mockReturnValue({
      data: CATALOG,
      isPending: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    })
    mocks.runWithReauth.mockImplementation((fn: () => Promise<unknown>) => fn())
    mocks.createRole.mockResolvedValue(CREATED)
    mocks.useBlocker.mockReturnValue({
      status: 'idle',
      proceed: undefined,
      reset: undefined,
    })
  })

  it('requires a name and at least one permission before submitting', async () => {
    const screen = await renderPage()

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('roles:new.submit') })
    )

    await expect
      .element(screen.getByText(i18n.t('validation:required')))
      .toBeInTheDocument()
    expect(mocks.createRole).not.toHaveBeenCalled()
  })

  it('creates the role behind reauthentication and opens its detail page', async () => {
    const screen = await renderPage()
    await fillNameAndPermission(screen)

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('roles:new.submit') })
    )

    expect(mocks.runWithReauth).toHaveBeenCalledTimes(1)
    expect(mocks.createRole).toHaveBeenCalledWith({
      data: {
        name: 'support',
        permission_keys: ['users.read'],
      },
    })
    expect(mocks.navigate).toHaveBeenCalledWith({
      to: '/roles/$roleId',
      params: { roleId: CREATED.id },
    })
    expect(mocks.toastSuccess).toHaveBeenCalledWith(i18n.t('roles:new.success'))
  })

  it('maps a duplicate name back to the name field', async () => {
    mocks.createRole.mockRejectedValue(
      new ApiError({ code: 'role_name_taken', status: 409 })
    )
    const screen = await renderPage()
    await fillNameAndPermission(screen)

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('roles:new.submit') })
    )

    await expect
      .element(screen.getByText(i18n.t('errors:role.role_name_taken')))
      .toBeInTheDocument()
    expect(mocks.navigate).not.toHaveBeenCalled()
  })

  it('maps a server field error on the description', async () => {
    mocks.createRole.mockRejectedValue(
      new ApiError({
        code: 'validation_failed',
        status: 422,
        errors: [
          { pointer: '/description', code: 'max_length', params: { max: 500 } },
        ],
      })
    )
    const screen = await renderPage()
    await fillNameAndPermission(screen)

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('roles:new.submit') })
    )

    await expect
      .element(screen.getByText(i18n.t('validation:max_length', { max: 500 })))
      .toBeInTheDocument()
  })

  it('shows the permission error for an empty permission set from the server', async () => {
    mocks.createRole.mockRejectedValue(
      new ApiError({ code: 'empty_permissions', status: 422 })
    )
    const screen = await renderPage()
    await fillNameAndPermission(screen)

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('roles:new.submit') })
    )

    await expect
      .element(screen.getByText(i18n.t('errors:role.empty_permissions')))
      .toBeInTheDocument()
  })

  it('reports a permission catalog failure and retries the catalog', async () => {
    const refetch = vi.fn()
    mocks.useListPermissions.mockReturnValue({
      data: undefined,
      isPending: false,
      isError: true,
      error: new Error('boom'),
      refetch,
    })
    const screen = await renderPage()

    await expect
      .element(screen.getByText(i18n.t('roles:states.permissionsLoadError')))
      .toBeInTheDocument()

    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('roles:states.permissionsRetry'),
      })
    )
    expect(refetch).toHaveBeenCalledTimes(1)
  })
})

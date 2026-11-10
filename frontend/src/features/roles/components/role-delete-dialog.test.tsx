import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import type { RoleSummary, UserSelfResponse } from '@/api/generated/model'
import { ApiError } from '@/lib/api-error'
import { RoleDeleteDialog } from './role-delete-dialog'

const mocks = vi.hoisted(() => ({
  deleteRole: vi.fn(),
  runWithReauth: vi.fn(),
  navigate: vi.fn(),
  toastSuccess: vi.fn(),
}))

vi.mock('@/api/generated/rs-admin', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/api/generated/rs-admin')>()
  return {
    ...actual,
    useDeleteRole: () => ({
      mutateAsync: mocks.deleteRole,
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
  }
})

vi.mock('sonner', () => ({
  toast: {
    success: (...args: unknown[]) => mocks.toastSuccess(...args),
  },
}))

const ROLE: RoleSummary = {
  id: 'role-support',
  name: 'support',
  description: 'Support staff',
  is_builtin: false,
  permission_keys: ['users.read', 'roles.read'],
  assignable: true,
  manageable: true,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-02T00:00:00Z',
}

const ADMIN: UserSelfResponse = {
  id: 'user-admin',
  username: 'admin',
  email: 'admin@example.com',
  display_name: 'Admin',
  status: 'active',
  must_change_password: false,
  is_super_admin: false,
  permissions: ['roles.read', 'roles.delete', 'users.read'],
  roles: [],
}

function renderDialog({
  currentUser = ADMIN,
  onDeleted = vi.fn(),
}: {
  currentUser?: UserSelfResponse
  onDeleted?: () => void
} = {}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })

  return render(
    <QueryClientProvider client={queryClient}>
      <RoleDeleteDialog
        open
        role={ROLE}
        currentUser={currentUser}
        onOpenChange={vi.fn()}
        onDeleted={onDeleted}
      />
    </QueryClientProvider>
  )
}

describe('RoleDeleteDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.runWithReauth.mockImplementation((fn: () => Promise<unknown>) => fn())
    mocks.deleteRole.mockResolvedValue(undefined)
  })

  it('deletes behind reauthentication and reports success', async () => {
    const onDeleted = vi.fn()
    const screen = await renderDialog({ onDeleted })

    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('roles:delete.confirm'),
      })
    )

    expect(mocks.runWithReauth).toHaveBeenCalledTimes(1)
    expect(mocks.deleteRole).toHaveBeenCalledWith({ id: ROLE.id })
    expect(mocks.toastSuccess).toHaveBeenCalledWith(
      i18n.t('roles:delete.success')
    )
    expect(onDeleted).toHaveBeenCalledTimes(1)
  })

  it('explains a role_in_use conflict and links to the filtered user list', async () => {
    mocks.deleteRole.mockRejectedValue(
      new ApiError({ code: 'role_in_use', status: 409 })
    )
    const screen = await renderDialog()

    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('roles:delete.confirm'),
      })
    )

    await expect
      .element(screen.getByText(i18n.t('roles:delete.inUseTitle')))
      .toBeInTheDocument()

    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('roles:delete.viewUsers'),
      })
    )

    expect(mocks.navigate).toHaveBeenCalledWith({
      to: '/users',
      search: { role_id: ROLE.id },
    })
  })

  it('does not offer the user link without users.read', async () => {
    mocks.deleteRole.mockRejectedValue(
      new ApiError({ code: 'role_in_use', status: 409 })
    )
    const screen = await renderDialog({
      currentUser: { ...ADMIN, permissions: ['roles.read', 'roles.delete'] },
    })

    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('roles:delete.confirm'),
      })
    )

    await expect
      .element(screen.getByText(i18n.t('roles:delete.inUseTitle')))
      .toBeInTheDocument()
    await expect
      .element(
        screen.getByRole('button', {
          name: i18n.t('roles:delete.viewUsers'),
        })
      )
      .not.toBeInTheDocument()
  })
})

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import type {
  RoleSummary,
  UserCreatedResponse,
  UserSelfResponse,
} from '@/api/generated/model'
import { ApiError } from '@/lib/api-error'
import { UserCreateDialog } from './user-create-dialog'

const mocks = vi.hoisted(() => ({
  createUser: vi.fn(),
  reset: vi.fn(),
  runWithReauth: vi.fn(),
}))

vi.mock('@/api/generated/rs-admin', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/api/generated/rs-admin')>()
  return {
    ...actual,
    useCreateUser: () => ({
      mutateAsync: mocks.createUser,
      reset: mocks.reset,
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

const CURRENT_USER: UserSelfResponse = {
  id: 'user-admin',
  username: 'admin',
  email: 'admin@example.com',
  display_name: 'Admin',
  status: 'active',
  must_change_password: false,
  is_super_admin: true,
  permissions: ['users.create', 'users.assign_role', 'roles.read'],
  roles: [],
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
    description: 'Support staff',
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
    assignable: true,
    manageable: false,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
  },
]

const CREATED: UserCreatedResponse = {
  user: {
    id: 'user-new',
    username: 'newuser',
    email: 'new@example.com',
    display_name: 'newuser',
    status: 'active',
    roles: [],
    must_change_password: true,
    locked_until: null,
    last_login_at: null,
    temporary_password_expires_at: '2026-11-12T00:00:00Z',
    created_at: '2026-11-09T00:00:00Z',
    updated_at: '2026-11-09T00:00:00Z',
  },
  temporary_password: 'generated-temp-password',
  temporary_password_expires_at: '2026-11-12T00:00:00Z',
}

function renderDialog({
  open = true,
  canAssignRoles = true,
  onOpenChange = vi.fn(),
}: {
  open?: boolean
  canAssignRoles?: boolean
  onOpenChange?: (open: boolean) => void
} = {}) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  })

  return render(
    <QueryClientProvider client={queryClient}>
      <UserCreateDialog
        open={open}
        onOpenChange={onOpenChange}
        canAssignRoles={canAssignRoles}
        currentUser={CURRENT_USER}
        roleOptions={ROLES}
      />
    </QueryClientProvider>
  )
}

async function fillRequiredFields(
  screen: Awaited<ReturnType<typeof renderDialog>>
) {
  await userEvent.fill(
    screen.getByRole('textbox', { name: i18n.t('users:create.username') }),
    ' NewUser '
  )
  await userEvent.fill(
    screen.getByRole('textbox', { name: i18n.t('users:create.email') }),
    ' NEW@EXAMPLE.COM '
  )
}

describe('UserCreateDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.runWithReauth.mockImplementation((fn: () => Promise<unknown>) => fn())
    mocks.createUser.mockResolvedValue(CREATED)
  })

  it('creates with a generated password and reveals it only until the panel closes', async () => {
    const onOpenChange = vi.fn()
    const screen = await renderDialog({ onOpenChange })

    await fillRequiredFields(screen)
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('users:create.submit') })
    )

    await vi.waitFor(() => {
      expect(mocks.createUser).toHaveBeenCalledTimes(1)
    })
    expect(mocks.createUser.mock.calls[0][0]).toEqual({
      data: { username: 'newuser', email: 'new@example.com' },
    })
    expect(mocks.runWithReauth).toHaveBeenCalledTimes(1)

    const password = screen.getByLabelText(i18n.t('users:password.title'))
    await expect.element(password).toHaveValue('generated-temp-password')

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('users:password.saved') })
    )
    expect(onOpenChange).toHaveBeenCalledWith(false)

    await screen.rerender(
      <QueryClientProvider client={new QueryClient()}>
        <UserCreateDialog
          open={false}
          onOpenChange={onOpenChange}
          canAssignRoles
          currentUser={CURRENT_USER}
          roleOptions={ROLES}
        />
      </QueryClientProvider>
    )
    await screen.rerender(
      <QueryClientProvider client={new QueryClient()}>
        <UserCreateDialog
          open
          onOpenChange={onOpenChange}
          canAssignRoles
          currentUser={CURRENT_USER}
          roleOptions={ROLES}
        />
      </QueryClientProvider>
    )

    await expect
      .element(screen.getByLabelText(i18n.t('users:password.title')))
      .not.toBeInTheDocument()
  })

  it('hides the role picker and omits role_ids without both role permissions', async () => {
    const screen = await renderDialog({ canAssignRoles: false })

    await fillRequiredFields(screen)
    expect(
      screen.getByRole('checkbox', { name: /support/ })
    ).not.toBeInTheDocument()

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('users:create.submit') })
    )

    await vi.waitFor(() => {
      expect(mocks.createUser).toHaveBeenCalledTimes(1)
    })
    expect(mocks.createUser.mock.calls[0][0].data.role_ids).toBeUndefined()
  })

  it('submits only extra selected roles and never the baseline', async () => {
    const screen = await renderDialog()

    await fillRequiredFields(screen)
    await userEvent.click(screen.getByRole('checkbox', { name: /support/ }))
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('users:create.submit') })
    )

    await vi.waitFor(() => {
      expect(mocks.createUser).toHaveBeenCalledTimes(1)
    })
    expect(mocks.createUser.mock.calls[0][0].data.role_ids).toEqual([
      'role-support',
    ])
  })

  it('maps identifier_taken to the matching field', async () => {
    mocks.createUser.mockRejectedValueOnce(
      new ApiError({
        code: 'identifier_taken',
        status: 409,
        errors: [{ pointer: '/email', code: 'identifier_taken' }],
      })
    )
    const screen = await renderDialog()

    await fillRequiredFields(screen)
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('users:create.submit') })
    )

    await expect
      .element(screen.getByText(i18n.t('errors:user.identifier_taken')))
      .toBeInTheDocument()
  })

  it('requires matching manual passwords and submits the supplied value', async () => {
    const screen = await renderDialog()

    await fillRequiredFields(screen)
    await userEvent.click(
      screen.getByRole('radio', { name: i18n.t('users:create.manual') })
    )
    await userEvent.fill(
      screen.getByLabelText(i18n.t('users:create.password'), {
        exact: true,
      }),
      'a-long-enough-password'
    )
    await userEvent.fill(
      screen.getByLabelText(i18n.t('users:create.confirmPassword')),
      'different-password'
    )
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('users:create.submit') })
    )

    await expect
      .element(screen.getByText(i18n.t('users:create.passwordMismatch')))
      .toBeInTheDocument()
    expect(mocks.createUser).not.toHaveBeenCalled()

    await userEvent.fill(
      screen.getByLabelText(i18n.t('users:create.confirmPassword')),
      'a-long-enough-password'
    )
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('users:create.submit') })
    )

    await vi.waitFor(() => {
      expect(mocks.createUser).toHaveBeenCalledTimes(1)
    })
    expect(mocks.createUser.mock.calls[0][0].data.password).toBe(
      'a-long-enough-password'
    )
  })
})

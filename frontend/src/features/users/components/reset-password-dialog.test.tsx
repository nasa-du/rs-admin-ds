import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import type { UserCreatedResponse, UserDetail } from '@/api/generated/model'
import {
  getGetUserQueryKey,
  getListUsersQueryKey,
} from '@/api/generated/rs-admin'
import { ApiError } from '@/lib/api-error'
import { ResetPasswordDialog } from './reset-password-dialog'

const mocks = vi.hoisted(() => ({
  resetPassword: vi.fn(),
  runWithReauth: vi.fn(),
}))

vi.mock('@/api/generated/rs-admin', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/api/generated/rs-admin')>()
  return {
    ...actual,
    useResetUserPassword: () => ({
      mutateAsync: mocks.resetPassword,
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

const USER: Pick<UserDetail, 'id' | 'username'> = {
  id: 'user-alice',
  username: 'alice',
}

const ISSUED: UserCreatedResponse = {
  user: {
    id: 'user-alice',
    username: 'alice',
    email: 'alice@example.com',
    display_name: 'Alice',
    status: 'active',
    roles: [],
    must_change_password: true,
    locked_until: null,
    last_login_at: null,
    temporary_password_expires_at: '2026-11-12T00:00:00Z',
    created_at: '2026-01-02T03:04:05Z',
    updated_at: '2026-01-03T03:04:05Z',
  },
  temporary_password: 'reset-temp-password',
  temporary_password_expires_at: '2026-11-12T00:00:00Z',
}

async function renderDialog({
  open = true,
  onOpenChange = vi.fn(),
}: {
  open?: boolean
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
      <ResetPasswordDialog
        open={open}
        onOpenChange={onOpenChange}
        user={USER}
      />
    </QueryClientProvider>
  )

  return { screen, queryClient, invalidate, onOpenChange }
}

describe('ResetPasswordDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.runWithReauth.mockImplementation((fn: () => Promise<unknown>) => fn())
    mocks.resetPassword.mockResolvedValue(ISSUED)
  })

  it('generates a password, runs behind reauthentication, and reveals it once', async () => {
    const { screen, queryClient, invalidate, onOpenChange } =
      await renderDialog()

    await expect
      .element(
        screen.getByText(i18n.t('users:manage.resetPassword.consequences'))
      )
      .toBeInTheDocument()

    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('users:manage.resetPassword.submit'),
      })
    )

    await vi.waitFor(() => {
      expect(mocks.resetPassword).toHaveBeenCalledWith({
        id: USER.id,
        data: {},
      })
    })
    expect(mocks.runWithReauth).toHaveBeenCalledTimes(1)
    expect(invalidate).toHaveBeenCalledWith(
      expect.objectContaining({ queryKey: getGetUserQueryKey(USER.id) })
    )
    expect(invalidate).toHaveBeenCalledWith(
      expect.objectContaining({ queryKey: getListUsersQueryKey() })
    )
    expect(onOpenChange).not.toHaveBeenCalled()

    await expect
      .element(
        screen.getByLabelText(i18n.t('users:password.title'), { exact: true })
      )
      .toHaveValue('reset-temp-password')

    // The one-time secret stays in the dialog only; it must never enter the
    // React Query cache where a later read could recover it (spec §9.9).
    const cached = JSON.stringify(queryClient.getQueryCache().getAll())
    expect(cached).not.toContain('reset-temp-password')

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('users:password.saved') })
    )
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })

  it('asks for confirmation before closing an unsaved temporary password', async () => {
    const { screen } = await renderDialog()

    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('users:manage.resetPassword.submit'),
      })
    )
    await expect
      .element(
        screen.getByLabelText(i18n.t('users:password.title'), { exact: true })
      )
      .toHaveValue('reset-temp-password')

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('common:close') })
    )

    await expect
      .element(screen.getByText(i18n.t('users:password.closeTitle')))
      .toBeInTheDocument()
  })

  it('submits a manual password only when the confirmation matches', async () => {
    const { screen } = await renderDialog()

    await userEvent.click(
      screen.getByRole('radio', {
        name: i18n.t('users:manage.resetPassword.manual'),
      })
    )
    await userEvent.fill(
      screen.getByLabelText(i18n.t('users:manage.resetPassword.password'), {
        exact: true,
      }),
      'a-long-enough-password'
    )
    await userEvent.fill(
      screen.getByLabelText(
        i18n.t('users:manage.resetPassword.confirmPassword')
      ),
      'different-password'
    )
    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('users:manage.resetPassword.submit'),
      })
    )

    await expect
      .element(screen.getByText(i18n.t('validation:passwordMismatch')))
      .toBeInTheDocument()
    expect(mocks.resetPassword).not.toHaveBeenCalled()

    await userEvent.fill(
      screen.getByLabelText(
        i18n.t('users:manage.resetPassword.confirmPassword')
      ),
      'a-long-enough-password'
    )
    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('users:manage.resetPassword.submit'),
      })
    )

    await vi.waitFor(() => {
      expect(mocks.resetPassword).toHaveBeenCalledWith({
        id: USER.id,
        data: { password: 'a-long-enough-password' },
      })
    })
  })

  it('maps a server password-policy rejection to the password field', async () => {
    mocks.resetPassword.mockRejectedValueOnce(
      new ApiError({
        code: 'password_policy_violation',
        status: 422,
        errors: [
          {
            pointer: '/password',
            code: 'password_policy_violation',
            params: { reason: 'blocklisted' },
          },
        ],
      })
    )
    const { screen } = await renderDialog()

    await userEvent.click(
      screen.getByRole('radio', {
        name: i18n.t('users:manage.resetPassword.manual'),
      })
    )
    await userEvent.fill(
      screen.getByLabelText(i18n.t('users:manage.resetPassword.password'), {
        exact: true,
      }),
      'a-long-enough-password'
    )
    await userEvent.fill(
      screen.getByLabelText(
        i18n.t('users:manage.resetPassword.confirmPassword')
      ),
      'a-long-enough-password'
    )
    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('users:manage.resetPassword.submit'),
      })
    )

    await expect
      .element(screen.getByText(i18n.t('validation:password.blocklisted')))
      .toBeInTheDocument()
  })
})

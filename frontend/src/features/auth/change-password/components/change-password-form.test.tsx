import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import { apiClient } from '@/api/mutator'
import { authSession } from '@/features/auth/auth-session'
import { ChangePasswordForm } from './change-password-form'

const navigate = vi.fn()

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>()
  return { ...actual, useNavigate: () => navigate }
})

function problem(status: number, code: string, extra: object = {}) {
  return {
    response: {
      status,
      data: { code, status, request_id: 'req-cp-1', ...extra },
      headers: {},
    },
  }
}

function renderForm(props: {
  redirect?: string
  mustChangePassword?: boolean
}) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  })

  return render(
    <QueryClientProvider client={queryClient}>
      <ChangePasswordForm {...props} />
    </QueryClientProvider>
  )
}

async function fillAndSubmit(
  screen: Awaited<ReturnType<typeof renderForm>>,
  values = {
    currentPassword: 'current password',
    newPassword: 'a new password that is long enough',
    confirmPassword: 'a new password that is long enough',
  }
) {
  await userEvent.fill(
    screen.getByLabelText(i18n.t('auth:changePassword.currentPassword'), {
      exact: true,
    }),
    values.currentPassword
  )
  await userEvent.fill(
    screen.getByLabelText(i18n.t('auth:changePassword.newPassword'), {
      exact: true,
    }),
    values.newPassword
  )
  await userEvent.fill(
    screen.getByLabelText(i18n.t('auth:changePassword.confirmPassword'), {
      exact: true,
    }),
    values.confirmPassword
  )
  await userEvent.click(
    screen.getByRole('button', { name: i18n.t('auth:changePassword.submit') })
  )
}

beforeEach(() => {
  vi.clearAllMocks()
})

describe('ChangePasswordForm', () => {
  it('submits the password change, updates the session, and returns to the target', async () => {
    const request = vi.spyOn(apiClient, 'request').mockResolvedValue({
      data: {
        access_token: 'rotated-token',
        token_type: 'Bearer',
        expires_in: 900,
      },
    } as never)
    const applyAccessToken = vi
      .spyOn(authSession, 'applyAccessToken')
      .mockImplementation(() => undefined)
    const refreshMe = vi
      .spyOn(authSession, 'refreshMe')
      .mockResolvedValue(undefined as never)
    const screen = await renderForm({ redirect: '/users?page=2' })

    await fillAndSubmit(screen)

    await vi.waitFor(() =>
      expect(navigate).toHaveBeenCalledWith({
        href: '/users?page=2',
        replace: true,
      })
    )
    expect(request).toHaveBeenCalledWith(
      expect.objectContaining({
        url: '/api/v1/auth/change-password',
        method: 'POST',
        data: {
          current_password: 'current password',
          new_password: 'a new password that is long enough',
        },
      })
    )
    expect(applyAccessToken).toHaveBeenCalledWith('rotated-token')
    expect(refreshMe).toHaveBeenCalledTimes(1)
  })

  it('still leaves the page when refreshing /me fails after a successful change', async () => {
    vi.spyOn(apiClient, 'request').mockResolvedValue({
      data: {
        access_token: 'rotated-token',
        token_type: 'Bearer',
        expires_in: 900,
      },
    } as never)
    vi.spyOn(authSession, 'applyAccessToken').mockImplementation(
      () => undefined
    )
    vi.spyOn(authSession, 'refreshMe').mockRejectedValue(
      new TypeError('Failed to fetch')
    )
    const screen = await renderForm({ redirect: '/users' })

    await fillAndSubmit(screen)

    await vi.waitFor(() =>
      expect(navigate).toHaveBeenCalledWith({
        href: '/users',
        replace: true,
      })
    )
  })

  it('rejects a mismatched confirmation before calling the API', async () => {
    const request = vi.spyOn(apiClient, 'request')
    const screen = await renderForm({})

    await fillAndSubmit(screen, {
      currentPassword: 'current password',
      newPassword: 'a new password that is long enough',
      confirmPassword: 'a different password that is long',
    })

    await expect
      .element(screen.getByText(i18n.t('validation:passwordMismatch')))
      .toBeInTheDocument()
    expect(request).not.toHaveBeenCalled()
  })

  it('attaches invalid_current_password to the current password field', async () => {
    vi.spyOn(apiClient, 'request').mockRejectedValue(
      problem(422, 'invalid_current_password')
    )
    const screen = await renderForm({})

    await fillAndSubmit(screen)

    await expect
      .element(screen.getByText(i18n.t('errors:auth.invalid_current_password')))
      .toBeInTheDocument()
    expect(navigate).not.toHaveBeenCalled()
  })

  it('maps password policy failures to the new password field', async () => {
    vi.spyOn(apiClient, 'request').mockRejectedValue(
      problem(422, 'password_policy_violation', {
        errors: [
          {
            pointer: '/new_password',
            code: 'password_policy_violation',
            params: { reason: 'too_short' },
          },
        ],
      })
    )
    const screen = await renderForm({})

    await fillAndSubmit(screen)

    await expect
      .element(screen.getByText(i18n.t('validation:password.too_short')))
      .toBeInTheDocument()
  })

  it('disables submit during a 429 countdown', async () => {
    vi.spyOn(apiClient, 'request').mockRejectedValue({
      response: {
        status: 429,
        data: { code: 'rate_limited', status: 429 },
        headers: { 'retry-after': '1' },
      },
    })
    const screen = await renderForm({})

    await fillAndSubmit(screen)

    const submitButton = screen.getByRole('button', {
      name: i18n.t('auth:changePassword.submit'),
    })
    await expect
      .element(screen.getByRole('alert'))
      .toHaveTextContent(i18n.t('errors:common.rate_limited'))
    await expect.element(submitButton).toBeDisabled()

    await new Promise((resolve) => setTimeout(resolve, 1100))
    await expect.element(submitButton).toBeEnabled()
  })

  it('shows a copyable request id for server failures', async () => {
    vi.spyOn(apiClient, 'request').mockRejectedValue({
      response: {
        status: 500,
        data: {
          code: 'internal_error',
          status: 500,
          request_id: 'req-cp-500',
        },
        headers: {},
      },
    })
    const screen = await renderForm({})

    await fillAndSubmit(screen)

    await expect
      .element(screen.getByRole('alert'))
      .toHaveTextContent(i18n.t('errors:common.internal_error'))
    await expect.element(screen.getByText('req-cp-500')).toBeInTheDocument()
  })

  it('explains why the change is mandatory for a restricted session', async () => {
    const screen = await renderForm({ mustChangePassword: true })

    await expect
      .element(
        screen.getByText(i18n.t('auth:changePassword.forcedDescription'))
      )
      .toBeInTheDocument()
  })
})

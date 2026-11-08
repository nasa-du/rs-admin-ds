import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import type { UserSelfResponse } from '@/api/generated/model'
import { apiClient } from '@/api/mutator'
import { authStore } from '@/features/auth/auth-store'
import { UserAuthForm } from './user-auth-form'

const navigate = vi.fn()

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>()
  return { ...actual, useNavigate: () => navigate }
})

const USER: UserSelfResponse = {
  id: 'user-1',
  username: 'alice',
  email: 'alice@example.com',
  display_name: 'Alice',
  status: 'active',
  must_change_password: false,
  is_super_admin: false,
  permissions: ['users.read'],
  roles: [],
}

function loginSuccess(user: UserSelfResponse = USER) {
  return {
    data: {
      access_token: 'login-token',
      token_type: 'Bearer',
      expires_in: 900,
      user,
    },
  } as never
}

function problem(status: number, code: string, extra: object = {}) {
  return {
    response: {
      status,
      data: { code, status, request_id: 'req-123', ...extra },
      headers: {},
    },
  }
}

function renderForm(redirect?: string) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  })

  return render(
    <QueryClientProvider client={queryClient}>
      <UserAuthForm redirect={redirect} />
    </QueryClientProvider>
  )
}

async function submit(
  screen: Awaited<ReturnType<typeof renderForm>>,
  identifier = 'alice',
  password = 'correct horse battery staple'
) {
  await userEvent.fill(
    screen.getByRole('textbox', { name: i18n.t('auth:signIn.identifier') }),
    identifier
  )
  await userEvent.fill(
    screen.getByLabelText(i18n.t('auth:signIn.password')),
    password
  )
  await userEvent.click(
    screen.getByRole('button', { name: i18n.t('auth:signIn.submit') })
  )
}

beforeEach(() => {
  authStore.getState().reset()
  vi.clearAllMocks()
})

describe('UserAuthForm', () => {
  it('renders only the identifier and password fields', async () => {
    const screen = await renderForm()

    await expect
      .element(
        screen.getByRole('textbox', { name: i18n.t('auth:signIn.identifier') })
      )
      .toBeInTheDocument()
    await expect
      .element(screen.getByLabelText(i18n.t('auth:signIn.password')))
      .toBeInTheDocument()
    await expect.element(screen.getByText(/Sign up/i)).not.toBeInTheDocument()
    await expect
      .element(screen.getByText(/Forgot password/i))
      .not.toBeInTheDocument()
  })

  it('signs in and navigates to the sanitized redirect target', async () => {
    const request = vi
      .spyOn(apiClient, 'request')
      .mockResolvedValue(loginSuccess())
    const screen = await renderForm('/users?page=2')

    await submit(screen)

    await vi.waitFor(() =>
      expect(navigate).toHaveBeenCalledWith({
        href: '/users?page=2',
        replace: true,
      })
    )
    expect(request).toHaveBeenCalledWith(
      expect.objectContaining({
        url: '/api/v1/auth/login',
        method: 'POST',
        data: {
          identifier: 'alice',
          password: 'correct horse battery staple',
        },
      })
    )
    expect(authStore.getState().accessToken).toBe('login-token')
  })

  it('rejects an off-site redirect and falls back to the dashboard', async () => {
    vi.spyOn(apiClient, 'request').mockResolvedValue(loginSuccess())
    const screen = await renderForm('//evil.example/steal')

    await submit(screen)

    await vi.waitFor(() =>
      expect(navigate).toHaveBeenCalledWith({ href: '/', replace: true })
    )
  })

  it('sends a restricted session to change-password with the target', async () => {
    vi.spyOn(apiClient, 'request').mockResolvedValue(
      loginSuccess({ ...USER, must_change_password: true })
    )
    const screen = await renderForm('/users')

    await submit(screen)

    await vi.waitFor(() =>
      expect(navigate).toHaveBeenCalledWith({
        to: '/change-password',
        search: { redirect: '/users' },
        replace: true,
      })
    )
  })

  it('maps invalid credentials to a generic message without navigating', async () => {
    vi.spyOn(apiClient, 'request').mockRejectedValue(
      problem(401, 'invalid_credentials')
    )
    const screen = await renderForm()

    await submit(screen)

    await expect
      .element(screen.getByRole('alert'))
      .toHaveTextContent(i18n.t('errors:auth.invalid_credentials'))
    expect(navigate).not.toHaveBeenCalled()
  })

  it('maps account and temporary-password refusals by code', async () => {
    vi.spyOn(apiClient, 'request').mockRejectedValue(
      problem(403, 'temporary_password_expired')
    )
    const screen = await renderForm()

    await submit(screen)

    await expect
      .element(screen.getByRole('alert'))
      .toHaveTextContent(i18n.t('errors:auth.temporary_password_expired'))
  })

  it('disables submit during the Retry-After countdown of a 429', async () => {
    vi.spyOn(apiClient, 'request').mockRejectedValue({
      response: {
        status: 429,
        data: { code: 'login_locked', status: 429 },
        headers: { 'retry-after': '1' },
      },
    })
    const screen = await renderForm()

    await submit(screen)

    const submitButton = screen.getByRole('button', {
      name: i18n.t('auth:signIn.submit'),
    })
    await expect
      .element(screen.getByRole('alert'))
      .toHaveTextContent(i18n.t('errors:auth.login_locked'))
    await expect.element(submitButton).toBeDisabled()

    await new Promise((resolve) => setTimeout(resolve, 1100))
    await expect.element(submitButton).toBeEnabled()
  })

  it('shows the connection message for a network failure', async () => {
    vi.spyOn(apiClient, 'request').mockRejectedValue(
      new TypeError('Failed to fetch')
    )
    const screen = await renderForm()

    await submit(screen)

    await expect
      .element(screen.getByRole('alert'))
      .toHaveTextContent(i18n.t('errors:common.network_error'))
  })

  it('shows a copyable request id for server failures', async () => {
    vi.spyOn(apiClient, 'request').mockRejectedValue({
      response: {
        status: 500,
        data: {
          code: 'internal_error',
          status: 500,
          request_id: 'req-abc-123',
        },
        headers: {},
      },
    })
    const screen = await renderForm()

    await submit(screen)

    await expect
      .element(screen.getByRole('alert'))
      .toHaveTextContent(i18n.t('errors:common.internal_error'))
    await expect.element(screen.getByText('req-abc-123')).toBeInTheDocument()
    await expect
      .element(
        screen.getByRole('button', { name: i18n.t('common:requestId.copy') })
      )
      .toBeInTheDocument()
  })

  it('maps 422 field errors onto the matching inputs', async () => {
    vi.spyOn(apiClient, 'request').mockRejectedValue(
      problem(422, 'validation_failed', {
        errors: [{ pointer: '/identifier', code: 'required' }],
      })
    )
    const screen = await renderForm()

    await submit(screen)

    await expect
      .element(screen.getByText(i18n.t('validation:required')))
      .toBeInTheDocument()
  })
})

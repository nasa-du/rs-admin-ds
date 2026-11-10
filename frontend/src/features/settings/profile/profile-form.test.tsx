import { QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { createAuthQueryClient, makeUser } from '@/test-utils/auth'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import { apiClient } from '@/api/mutator'
import { authKeys } from '@/features/auth/auth-session'
import { ProfileForm } from './profile-form'

function problem(status: number, code: string, extra: object = {}) {
  return {
    response: {
      status,
      data: { code, status, request_id: 'req-profile-1', ...extra },
      headers: {},
    },
  }
}

async function renderProfile(overrides = {}) {
  const user = makeUser({
    username: 'alice',
    display_name: 'Alice',
    email: 'alice@example.com',
    phone: '+1 555 0100',
    roles: [{ id: 'role-1', name: 'user', is_builtin: true }],
    ...overrides,
  })
  const queryClient = createAuthQueryClient(user)

  const screen = await render(
    <QueryClientProvider client={queryClient}>
      <ProfileForm />
    </QueryClientProvider>
  )

  return { screen, queryClient, user }
}

beforeEach(() => {
  vi.restoreAllMocks()
})

describe('ProfileForm', () => {
  it('edits only the display name and shows administrator-managed fields read-only', async () => {
    const { screen } = await renderProfile()

    const displayName = screen.getByLabelText(
      i18n.t('settings:profile.displayName')
    )
    await expect.element(displayName).toHaveValue('Alice')

    for (const [label, value] of [
      ['settings:profile.username', 'alice'],
      ['settings:profile.email', 'alice@example.com'],
      ['settings:profile.phone', '+1 555 0100'],
    ] as const) {
      const input = screen.getByLabelText(i18n.t(label))
      await expect.element(input).toHaveValue(value)
      await expect.element(input).toHaveAttribute('readonly')
    }
    await expect
      .element(screen.getByText(i18n.t('settings:profile.roles')))
      .toBeInTheDocument()
    await expect
      .element(screen.getByText(i18n.t('roles:builtin.user'), { exact: true }))
      .toBeInTheDocument()
    await expect
      .element(screen.getByText('user', { exact: true }))
      .toBeInTheDocument()
    await expect
      .element(screen.getByText(i18n.t('users:status.active'), { exact: true }))
      .toBeInTheDocument()
    expect(
      screen.getByText(i18n.t('settings:profile.adminManaged')).all()
    ).toHaveLength(3)
  })

  it('saves the display name and updates the auth query cache', async () => {
    const updated = makeUser({ display_name: 'Alice Chen' })
    const request = vi.spyOn(apiClient, 'request').mockResolvedValue({
      data: updated,
    } as never)
    const { screen, queryClient } = await renderProfile()

    await userEvent.fill(
      screen.getByLabelText(i18n.t('settings:profile.displayName')),
      'Alice Chen'
    )
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('settings:profile.submit') })
    )

    await vi.waitFor(() =>
      expect(request).toHaveBeenCalledWith(
        expect.objectContaining({
          url: '/api/v1/auth/me',
          method: 'PATCH',
          data: { display_name: 'Alice Chen' },
        })
      )
    )
    await vi.waitFor(() =>
      expect(queryClient.getQueryData(authKeys.me)).toEqual(updated)
    )
  })

  it('does not submit an empty display name', async () => {
    const request = vi.spyOn(apiClient, 'request')
    const { screen } = await renderProfile()

    await userEvent.fill(
      screen.getByLabelText(i18n.t('settings:profile.displayName')),
      ''
    )
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('settings:profile.submit') })
    )

    await expect
      .element(screen.getByText(i18n.t('validation:required')))
      .toBeInTheDocument()
    expect(request).not.toHaveBeenCalled()
  })

  it('maps a server validation failure to the display name field', async () => {
    vi.spyOn(apiClient, 'request').mockRejectedValue(
      problem(422, 'validation_failed', {
        errors: [
          {
            pointer: '/display_name',
            code: 'invalid_length',
            params: null,
          },
        ],
      })
    )
    const { screen } = await renderProfile()

    await userEvent.fill(
      screen.getByLabelText(i18n.t('settings:profile.displayName')),
      'x'
    )
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('settings:profile.submit') })
    )

    await expect
      .element(screen.getByText(i18n.t('validation:invalid')))
      .toBeInTheDocument()
  })

  it('shows a copyable request id for server failures', async () => {
    vi.spyOn(apiClient, 'request').mockRejectedValue(
      problem(503, 'service_unavailable')
    )
    const { screen } = await renderProfile()

    await userEvent.fill(
      screen.getByLabelText(i18n.t('settings:profile.displayName')),
      'Alice Chen'
    )
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('settings:profile.submit') })
    )

    await expect
      .element(screen.getByRole('alert'))
      .toHaveTextContent(i18n.t('errors:common.service_unavailable'))
    await expect.element(screen.getByText('req-profile-1')).toBeInTheDocument()
  })
})

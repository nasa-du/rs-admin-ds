import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import type { UserSelfResponse, UserSummary } from '@/api/generated/model'
import { getGetUserQueryKey } from '@/api/generated/rs-admin'
import { ApiError } from '@/lib/api-error'
import { UserEditDialog } from './user-edit-dialog'

const mocks = vi.hoisted(() => ({
  updateUser: vi.fn(),
  reset: vi.fn(),
  runWithReauth: vi.fn(),
}))

vi.mock('@/api/generated/rs-admin', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/api/generated/rs-admin')>()
  return {
    ...actual,
    useUpdateUser: () => ({
      mutateAsync: mocks.updateUser,
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
  permissions: ['users.update'],
  roles: [],
}

const USER: UserSummary = {
  id: 'user-alice',
  username: 'alice',
  email: 'alice@example.com',
  display_name: 'Alice',
  phone: '+15551234567',
  status: 'active',
  roles: [],
  must_change_password: false,
  locked_until: null,
  last_login_at: null,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
}

function renderDialog(onOpenChange = vi.fn()) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  })

  return render(
    <QueryClientProvider client={queryClient}>
      <UserEditDialog
        open
        onOpenChange={onOpenChange}
        user={USER}
        currentUser={CURRENT_USER}
      />
    </QueryClientProvider>
  )
}

function saveButton(screen: Awaited<ReturnType<typeof renderDialog>>) {
  return screen.getByRole('button', {
    name: i18n.t('users:edit.submit'),
  })
}

describe('UserEditDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.runWithReauth.mockImplementation((fn: () => Promise<unknown>) => fn())
    mocks.updateUser.mockResolvedValue({
      ...USER,
      display_name: 'Alice B.',
    })
  })

  it('keeps save disabled until a field is dirty and sends only that field', async () => {
    const onOpenChange = vi.fn()
    const screen = await renderDialog(onOpenChange)

    await expect.element(saveButton(screen)).toBeDisabled()

    await userEvent.fill(
      screen.getByRole('textbox', { name: i18n.t('users:edit.displayName') }),
      'Alice B.'
    )
    await expect.element(saveButton(screen)).toBeEnabled()
    await userEvent.click(saveButton(screen))

    await vi.waitFor(() => {
      expect(mocks.updateUser).toHaveBeenCalledTimes(1)
    })
    expect(mocks.updateUser.mock.calls[0][0]).toEqual({
      id: USER.id,
      data: { display_name: 'Alice B.' },
    })
    expect(mocks.runWithReauth).not.toHaveBeenCalled()
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })

  it('uses step-up and normalizes the identifier when username changes', async () => {
    const screen = await renderDialog()

    await userEvent.fill(
      screen.getByRole('textbox', { name: i18n.t('users:edit.username') }),
      '  BOB  '
    )
    await userEvent.click(saveButton(screen))

    await vi.waitFor(() => {
      expect(mocks.updateUser).toHaveBeenCalledTimes(1)
    })
    expect(mocks.runWithReauth).toHaveBeenCalledTimes(1)
    expect(mocks.updateUser.mock.calls[0][0].data).toEqual({
      username: 'bob',
    })
  })

  it('maps identifier_taken to the offending field and stays open', async () => {
    mocks.updateUser.mockRejectedValueOnce(
      new ApiError({
        code: 'identifier_taken',
        status: 409,
        errors: [{ pointer: '/username', code: 'identifier_taken' }],
      })
    )
    const onOpenChange = vi.fn()
    const screen = await renderDialog(onOpenChange)

    await userEvent.fill(
      screen.getByRole('textbox', { name: i18n.t('users:edit.username') }),
      'taken'
    )
    await userEvent.click(saveButton(screen))

    await expect
      .element(screen.getByText(i18n.t('errors:user.identifier_taken')))
      .toBeInTheDocument()
    expect(onOpenChange).not.toHaveBeenCalledWith(false)
  })

  it('invalidates the user query when the target is out of scope', async () => {
    mocks.updateUser.mockRejectedValueOnce(
      new ApiError({ code: 'out_of_scope', status: 403 })
    )
    const queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    })
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries')

    const screen = await render(
      <QueryClientProvider client={queryClient}>
        <UserEditDialog
          open
          onOpenChange={vi.fn()}
          user={USER}
          currentUser={CURRENT_USER}
        />
      </QueryClientProvider>
    )

    await userEvent.fill(
      screen.getByRole('textbox', { name: i18n.t('users:edit.displayName') }),
      'Changed'
    )
    await userEvent.click(saveButton(screen))

    await vi.waitFor(() => {
      expect(invalidate).toHaveBeenCalledWith({
        queryKey: getGetUserQueryKey(USER.id),
      })
    })
  })

  it('confirms before closing a dirty form', async () => {
    const onOpenChange = vi.fn()
    const screen = await renderDialog(onOpenChange)

    await userEvent.fill(
      screen.getByRole('textbox', { name: i18n.t('users:edit.displayName') }),
      'Changed'
    )
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('common:actions.cancel') })
    )

    await expect
      .element(screen.getByText(i18n.t('users:edit.discardTitle')))
      .toBeInTheDocument()
    expect(mocks.updateUser).not.toHaveBeenCalled()

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('users:edit.discard') })
    )
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })
})

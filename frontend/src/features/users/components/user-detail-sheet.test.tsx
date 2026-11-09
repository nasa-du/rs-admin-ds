import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import type { UserDetail, UserSelfResponse } from '@/api/generated/model'
import { ApiError } from '@/lib/api-error'
import { UserDetailSheet } from './user-detail-sheet'

const mocks = vi.hoisted(() => ({
  useGetUser: vi.fn(),
}))

vi.mock('@/api/generated/rs-admin', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/api/generated/rs-admin')>()
  return {
    ...actual,
    useGetUser: (...args: unknown[]) => mocks.useGetUser(...args),
  }
})

const DETAIL: UserDetail = {
  id: 'user-alice',
  username: 'alice',
  email: 'alice@example.com',
  display_name: 'Alice',
  phone: '+15551234567',
  status: 'active',
  roles: [
    { id: 'role-user', name: 'user', is_builtin: true },
    { id: 'role-support', name: 'support', is_builtin: false },
  ],
  must_change_password: false,
  locked_until: null,
  last_login_at: null,
  temporary_password_expires_at: null,
  created_at: '2026-01-02T03:04:05Z',
  updated_at: '2026-01-03T03:04:05Z',
}

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

function readyQuery(user: UserDetail = DETAIL) {
  return {
    data: user,
    isPending: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }
}

function renderSheet({
  userId = DETAIL.id,
  onClose = vi.fn(),
  onEdit = vi.fn(),
  canEdit = true,
  currentUser = CURRENT_USER,
}: {
  userId?: string
  onClose?: () => void
  onEdit?: (user: UserDetail) => void
  canEdit?: boolean
  currentUser?: UserSelfResponse | null
} = {}) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  })

  return render(
    <QueryClientProvider client={queryClient}>
      <UserDetailSheet
        userId={userId}
        onClose={onClose}
        onEdit={onEdit}
        canEdit={canEdit}
        currentUser={currentUser}
      />
    </QueryClientProvider>
  )
}

describe('UserDetailSheet', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.useGetUser.mockReturnValue(readyQuery())
  })

  it('renders the detail fetched from GET /users/{id}', async () => {
    const screen = await renderSheet()

    await expect
      .element(screen.getByText(i18n.t('users:detail.title')))
      .toBeInTheDocument()
    await expect
      .element(screen.getByText('Alice', { exact: true }))
      .toBeInTheDocument()
    await expect
      .element(screen.getByText('alice@example.com'))
      .toBeInTheDocument()
    await expect.element(screen.getByText('+15551234567')).toBeInTheDocument()
    await expect
      .element(screen.getByText(i18n.t('users:status.neverSignedIn')))
      .toBeInTheDocument()
    await expect.element(screen.getByText('support')).toBeInTheDocument()
    expect(mocks.useGetUser).toHaveBeenCalledWith(
      DETAIL.id,
      expect.objectContaining({
        query: expect.objectContaining({ enabled: true, staleTime: 0 }),
      })
    )
  })

  it('closes by clearing the URL parameter', async () => {
    const onClose = vi.fn()
    const screen = await renderSheet({ onClose })

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('common:close') })
    )

    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('shows a retryable error instead of stale detail', async () => {
    const refetch = vi.fn()
    mocks.useGetUser.mockReturnValue({
      data: undefined,
      isPending: false,
      isError: true,
      error: new ApiError({ code: 'service_unavailable', status: 503 }),
      refetch,
    })
    const screen = await renderSheet()

    await expect
      .element(screen.getByText(i18n.t('errors:common.service_unavailable')))
      .toBeInTheDocument()

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('common:error.retry') })
    )
    expect(refetch).toHaveBeenCalledTimes(1)
  })

  it('opens the edit dialog for another user', async () => {
    const onEdit = vi.fn()
    const screen = await renderSheet({ onEdit })

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('users:detail.edit') })
    )
    expect(onEdit).toHaveBeenCalledWith(DETAIL)
  })

  it('disables management actions on the current user', async () => {
    const onEdit = vi.fn()
    const screen = await renderSheet({
      onEdit,
      currentUser: { ...CURRENT_USER, id: DETAIL.id },
    })

    await expect
      .element(
        screen.getByRole('button', { name: i18n.t('users:detail.edit') })
      )
      .toBeDisabled()
    expect(onEdit).not.toHaveBeenCalled()
  })
})

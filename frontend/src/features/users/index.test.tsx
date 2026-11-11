import type { ReactNode } from 'react'
import { QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { createAuthQueryClient } from '@/test-utils/auth'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import type {
  UserDetail,
  UserListResponse,
  UserSelfResponse,
  UserSummary,
} from '@/api/generated/model'
import { Users } from './index'

const mocks = vi.hoisted(() => ({
  useListUsers: vi.fn(),
  useGetUser: vi.fn(),
  useCreateUser: vi.fn(),
  listRoles: vi.fn(),
  navigate: vi.fn(),
  search: {} as Record<string, unknown>,
  updateUser: vi.fn(),
  runWithReauth: vi.fn(),
}))

vi.mock('@/api/generated/rs-admin', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/api/generated/rs-admin')>()
  return {
    ...actual,
    useListUsers: (...args: unknown[]) => mocks.useListUsers(...args),
    useGetUser: (...args: unknown[]) => mocks.useGetUser(...args),
    useCreateUser: (...args: unknown[]) => mocks.useCreateUser(...args),
    useUpdateUser: (...args: unknown[]) => mocks.updateUser(...args),
    listRoles: (...args: unknown[]) => mocks.listRoles(...args),
  }
})

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>()
  return {
    ...actual,
    useSearch: () => mocks.search,
    useNavigate: () => mocks.navigate,
  }
})

vi.mock('@/components/layout/header', () => ({
  Header: ({ children }: { children: ReactNode }) => (
    <header>{children}</header>
  ),
}))
vi.mock('@/components/search', () => ({ Search: () => null }))
vi.mock('@/components/theme-switch', () => ({ ThemeSwitch: () => null }))
vi.mock('@/components/config-drawer', () => ({ ConfigDrawer: () => null }))
vi.mock('@/components/profile-dropdown', () => ({
  ProfileDropdown: () => null,
}))
vi.mock('@/features/auth/reauth/reauth', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/features/auth/reauth/reauth')>()
  return {
    ...actual,
    runWithReauth: (fn: () => Promise<unknown>) => mocks.runWithReauth(fn),
  }
})

const READER: UserSelfResponse = {
  id: 'user-reader',
  username: 'reader',
  email: 'reader@example.com',
  display_name: 'Reader',
  status: 'active',
  must_change_password: false,
  is_super_admin: false,
  permissions: ['users.read'],
  roles: [],
}

const CREATOR: UserSelfResponse = {
  ...READER,
  permissions: ['users.read', 'users.create'],
}

const USER: UserSummary = {
  id: 'user-alice',
  username: 'alice',
  email: 'alice@example.com',
  display_name: 'Alice',
  phone: null,
  status: 'active',
  roles: [],
  must_change_password: false,
  locked_until: null,
  last_login_at: null,
  created_at: '2026-01-02T03:04:05Z',
  updated_at: '2026-01-03T03:04:05Z',
}

const PAGE: UserListResponse = {
  items: [USER],
  total: 1,
  page: 1,
  page_size: 20,
}

const DETAIL: UserDetail = {
  ...USER,
  temporary_password_expires_at: null,
}

function renderPage(user: UserSelfResponse) {
  const queryClient = createAuthQueryClient(user)

  return render(
    <QueryClientProvider client={queryClient}>
      <Users />
    </QueryClientProvider>
  )
}

function searchUpdater(): (prev: Record<string, unknown>) => unknown {
  const call = mocks.navigate.mock.calls[0]?.[0] as
    { search: (prev: Record<string, unknown>) => unknown } | undefined
  if (!call) throw new Error('navigate was not called')
  return call.search
}

describe('Users page', () => {
  it('renders no delete or bulk-selection affordances', async () => {
    const screen = await renderPage(READER)

    await expect
      .element(screen.getByText('alice', { exact: true }))
      .toBeInTheDocument()
    expect(
      screen.container.querySelectorAll('input[type="checkbox"]')
    ).toHaveLength(0)
    const buttons = Array.from(screen.container.querySelectorAll('button')).map(
      (button) => button.textContent?.trim() ?? ''
    )
    expect(buttons.some((name) => /delete|bulk/i.test(name))).toBe(false)
  })

  beforeEach(() => {
    vi.clearAllMocks()
    mocks.search = {}
    mocks.useListUsers.mockReturnValue({
      data: PAGE,
      isPending: false,
      isFetching: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    })
    mocks.useGetUser.mockReturnValue({
      data: undefined,
      isPending: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    })
    mocks.useCreateUser.mockReturnValue({
      mutateAsync: vi.fn(),
      reset: vi.fn(),
      isPending: false,
    })
    mocks.listRoles.mockResolvedValue({
      items: [],
      total: 0,
      page: 1,
      page_size: 200,
    })
    mocks.runWithReauth.mockImplementation((fn: () => Promise<unknown>) => fn())
  })

  it('feeds the URL search into the server list query', async () => {
    mocks.search = {
      page: 2,
      page_size: 50,
      q: 'alice',
      status: 'active',
      sort: 'email',
    }

    await renderPage(READER)

    expect(mocks.useListUsers).toHaveBeenCalledWith(
      {
        q: 'alice',
        status: 'active',
        role_id: undefined,
        is_locked: undefined,
        must_change_password: undefined,
        page: 2,
        page_size: 50,
        sort: 'email',
      },
      expect.objectContaining({
        query: expect.objectContaining({
          placeholderData: expect.any(Function),
        }),
      })
    )
  })

  it('opens the detail sheet by pushing ?user=<id>', async () => {
    const screen = await renderPage(READER)

    await userEvent.click(screen.getByText('alice', { exact: true }))

    expect(mocks.navigate).toHaveBeenCalledTimes(1)
    const options = mocks.navigate.mock.calls[0][0] as {
      to: string
      replace?: boolean
    }
    expect(options.to).toBe('/users')
    expect(options.replace).not.toBe(true)
    expect(searchUpdater()({ page: 2 })).toEqual({
      page: 2,
      user: USER.id,
    })
  })

  it('closes the detail sheet by clearing ?user', async () => {
    mocks.search = { user: DETAIL.id }
    mocks.useGetUser.mockReturnValue({
      data: DETAIL,
      isPending: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    })
    const screen = await renderPage(READER)

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('common:close') })
    )

    expect(searchUpdater()({ user: DETAIL.id, page: 2 })).toEqual({
      user: undefined,
      page: 2,
    })
  })

  it('hides the create entry without users.create', async () => {
    const screen = await renderPage(READER)

    await expect
      .element(
        screen.getByRole('button', { name: i18n.t('users:actions.create') })
      )
      .not.toBeInTheDocument()
  })

  it('opens the create dialog with users.create', async () => {
    const screen = await renderPage(CREATOR)

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('users:actions.create') })
    )

    await expect
      .element(
        screen.getByRole('dialog', { name: i18n.t('users:create.title') })
      )
      .toBeInTheDocument()
  })
})

import { QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { createAuthQueryClient } from '@/test-utils/auth'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import type {
  RoleListResponse,
  RoleSummary,
  UserSelfResponse,
} from '@/api/generated/model'
import { Roles } from './roles-list'

const mocks = vi.hoisted(() => ({
  useListRoles: vi.fn(),
  navigate: vi.fn(),
  search: {} as Record<string, unknown>,
}))

vi.mock('@/api/generated/rs-admin', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/api/generated/rs-admin')>()
  return {
    ...actual,
    useListRoles: (...args: unknown[]) => mocks.useListRoles(...args),
    useDeleteRole: () => ({ mutateAsync: vi.fn(), isPending: false }),
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

const SUPER_ADMIN: RoleSummary = {
  id: 'role-super-admin',
  name: 'super_admin',
  description: null,
  is_builtin: true,
  permission_keys: ['users.read', 'roles.read'],
  assignable: true,
  manageable: false,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
}

const SUPPORT: RoleSummary = {
  id: 'role-support',
  name: 'support',
  description: 'Support staff',
  is_builtin: false,
  permission_keys: ['users.read', 'roles.read'],
  assignable: true,
  manageable: true,
  created_at: '2026-01-02T00:00:00Z',
  updated_at: '2026-01-02T00:00:00Z',
}

const LOCKED: RoleSummary = {
  id: 'role-other',
  name: 'other-team',
  description: null,
  is_builtin: false,
  permission_keys: ['audit.read'],
  assignable: false,
  manageable: false,
  created_at: '2026-01-03T00:00:00Z',
  updated_at: '2026-01-03T00:00:00Z',
}

const PAGE: RoleListResponse = {
  items: [SUPER_ADMIN, SUPPORT, LOCKED],
  total: 3,
  page: 1,
  page_size: 20,
}

const ADMIN: UserSelfResponse = {
  id: 'user-admin',
  username: 'admin',
  email: 'admin@example.com',
  display_name: 'Admin',
  status: 'active',
  must_change_password: false,
  is_super_admin: false,
  permissions: ['roles.read', 'roles.create', 'roles.update', 'roles.delete'],
  roles: [],
}

const READER: UserSelfResponse = {
  ...ADMIN,
  permissions: ['roles.read'],
}

function renderPage(user: UserSelfResponse = ADMIN) {
  const queryClient = createAuthQueryClient(user)

  return render(
    <QueryClientProvider client={queryClient}>
      <Roles />
    </QueryClientProvider>
  )
}

async function openActions(
  screen: Awaited<ReturnType<typeof renderPage>>,
  row: number
) {
  await userEvent.click(
    screen.getByRole('button', { name: i18n.t('roles:actions.more') }).nth(row)
  )
}

describe('Roles list', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.search = {}
    mocks.useListRoles.mockReturnValue({
      data: PAGE,
      isPending: false,
      isFetching: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    })
  })

  it('feeds the URL search into the server list query', async () => {
    mocks.search = {
      page: 2,
      page_size: 50,
      q: 'support',
      is_builtin: false,
      sort: '-updated_at',
    }

    await renderPage()

    expect(mocks.useListRoles).toHaveBeenCalledWith(
      {
        q: 'support',
        is_builtin: false,
        page: 2,
        page_size: 50,
        sort: '-updated_at',
      },
      expect.objectContaining({
        query: expect.objectContaining({
          placeholderData: expect.any(Function),
        }),
      })
    )
  })

  it('renders localized built-in names, type badges and permission counts', async () => {
    const screen = await renderPage()

    await expect
      .element(screen.getByText(i18n.t('roles:builtin.super_admin')))
      .toBeInTheDocument()
    await expect.element(screen.getByText('super_admin')).toBeInTheDocument()
    await expect
      .element(screen.getByText(i18n.t('roles:type.builtin')).first())
      .toBeInTheDocument()
    await expect
      .element(screen.getByText(i18n.t('roles:type.readOnly')))
      .toBeInTheDocument()
    await expect
      .element(
        screen
          .getByText(i18n.t('roles:list.permissionCount', { count: 2 }))
          .first()
      )
      .toBeInTheDocument()
  })

  it('opens a role detail page when a row is clicked', async () => {
    const screen = await renderPage()

    await userEvent.click(screen.getByText('support', { exact: true }))

    expect(mocks.navigate).toHaveBeenCalledWith({
      to: '/roles/$roleId',
      params: { roleId: SUPPORT.id },
    })
  })

  it('hides the create entry without roles.create', async () => {
    const screen = await renderPage(READER)

    await expect
      .element(
        screen.getByRole('button', { name: i18n.t('roles:actions.create') })
      )
      .not.toBeInTheDocument()
  })

  it('always offers the view action, even without write permissions', async () => {
    const screen = await renderPage(READER)
    await openActions(screen, 0)

    await expect
      .element(
        screen.getByRole('menuitem', { name: i18n.t('roles:actions.view') })
      )
      .toBeInTheDocument()
  })

  it('opens the create page with roles.create', async () => {
    const screen = await renderPage()

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('roles:actions.create') })
    )

    expect(mocks.navigate).toHaveBeenCalledWith({ to: '/roles/new' })
  })

  it('never shows edit or delete actions for built-in roles', async () => {
    const screen = await renderPage()
    await openActions(screen, 0)

    await expect
      .element(
        screen.getByRole('menuitem', { name: i18n.t('roles:actions.view') })
      )
      .toBeInTheDocument()
    await expect
      .element(
        screen.getByRole('menuitem', { name: i18n.t('roles:actions.edit') })
      )
      .not.toBeInTheDocument()
    await expect
      .element(
        screen.getByRole('menuitem', { name: i18n.t('roles:actions.delete') })
      )
      .not.toBeInTheDocument()
  })

  it('disables management actions on a role outside the operator scope', async () => {
    const screen = await renderPage()
    await openActions(screen, 2)

    await expect
      .element(
        screen.getByRole('menuitem', { name: i18n.t('roles:actions.edit') })
      )
      .toHaveAttribute('data-disabled')
    await expect
      .element(
        screen.getByRole('menuitem', { name: i18n.t('roles:actions.delete') })
      )
      .toHaveAttribute('data-disabled')
  })

  it('opens the delete confirmation for a manageable custom role', async () => {
    const screen = await renderPage()
    await openActions(screen, 1)

    await userEvent.click(
      screen.getByRole('menuitem', { name: i18n.t('roles:actions.delete') })
    )

    await expect
      .element(
        screen.getByRole('alertdialog', {
          name: i18n.t('roles:delete.title'),
        })
      )
      .toBeInTheDocument()
  })

  it('shows a retryable error instead of silently clearing the table', async () => {
    mocks.useListRoles.mockReturnValue({
      data: undefined,
      isPending: false,
      isFetching: false,
      isError: true,
      error: new Error('boom'),
      refetch: vi.fn(),
    })
    const screen = await renderPage()

    await expect
      .element(screen.getByText(i18n.t('roles:states.loadError')))
      .toBeInTheDocument()
  })
})

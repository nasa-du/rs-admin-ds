import type { ReactNode } from 'react'
import { QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { createAuthQueryClient, makeUser } from '@/test-utils/auth'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import type {
  AuditLogDetail,
  AuditLogListResponse,
  AuditLogSummary,
  UserSelfResponse,
} from '@/api/generated/model'
import { ApiError } from '@/lib/api-error'
import { AuditLogs } from './index'

const mocks = vi.hoisted(() => ({
  useListAuditLogs: vi.fn(),
  useGetAuditLog: vi.fn(),
  useListUsers: vi.fn(),
  navigate: vi.fn(),
  search: {} as Record<string, unknown>,
}))

vi.mock('@/api/generated/rs-admin', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/api/generated/rs-admin')>()
  return {
    ...actual,
    useListAuditLogs: (...args: unknown[]) => mocks.useListAuditLogs(...args),
    useGetAuditLog: (...args: unknown[]) => mocks.useGetAuditLog(...args),
    useListUsers: (...args: unknown[]) => mocks.useListUsers(...args),
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
vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn() },
}))

const ACTOR_ID = '0190a1b2-0000-7000-8000-000000000001'
const REQUEST_ID = '0190a1b2-0000-7000-8000-000000000002'
const LOG_ID = '0190a1b2-0000-7000-8000-000000000003'
const TARGET_ID = '0190a1b2-0000-7000-8000-000000000004'

const WINDOW = {
  from: '2026-10-11T12:00:00.000Z',
  to: '2026-11-10T12:00:00.000Z',
  range: '30d',
}

const READER: UserSelfResponse = makeUser({
  id: 'user-reader',
  username: 'reader',
  email: 'reader@example.com',
  display_name: 'Reader',
  permissions: ['audit.read', 'users.read', 'roles.read'],
})

const SUMMARY: AuditLogSummary = {
  id: LOG_ID,
  occurred_at: '2026-11-10T11:00:00.000Z',
  actor: { id: ACTOR_ID, label: 'alice' },
  action: 'authn_login_success',
  target: { type: 'session', id: TARGET_ID, label: null },
  result: 'success',
  severity: 'info',
  ip: '203.0.113.7',
  request_id: REQUEST_ID,
}

const LIST: AuditLogListResponse = {
  items: [SUMMARY],
  total: 1,
  page: 1,
  page_size: 20,
}

const DETAIL: AuditLogDetail = {
  ...SUMMARY,
  user_agent: 'Mozilla/5.0',
  details: {
    auth_method: 'password',
    permission_keys: ['audit.read', 'users.read'],
    password: 'leak-me',
    nested: { access_token: 'also-secret' },
  },
}

function renderPage(user: UserSelfResponse = READER) {
  const queryClient = createAuthQueryClient(user)

  return render(
    <QueryClientProvider client={queryClient}>
      <AuditLogs />
    </QueryClientProvider>
  )
}

function listOptions() {
  const calls = mocks.useListAuditLogs.mock.calls
  const options = calls[calls.length - 1]?.[1] as
    { query?: { placeholderData?: unknown } } | undefined
  return options
}

function searchFromCall(index = 0): Record<string, unknown> {
  const options = mocks.navigate.mock.calls[index]?.[0] as
    | {
        search?:
          | Record<string, unknown>
          | ((previous: Record<string, unknown>) => Record<string, unknown>)
      }
    | undefined
  if (!options?.search) throw new Error('navigate was not called with search')
  return typeof options.search === 'function'
    ? options.search(mocks.search)
    : options.search
}

describe('Audit logs page', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.search = { ...WINDOW }
    mocks.useListAuditLogs.mockReturnValue({
      data: LIST,
      isPending: false,
      isFetching: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    })
    mocks.useGetAuditLog.mockReturnValue({
      data: undefined,
      isPending: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    })
    mocks.useListUsers.mockReturnValue({
      data: undefined,
      isPending: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    })
  })

  it('feeds URL filters and the absolute window into the list query', async () => {
    mocks.search = {
      ...WINDOW,
      page: 2,
      page_size: 50,
      actor_id: ACTOR_ID,
      action: 'authn_login_success',
      target_type: 'user',
      target_id: TARGET_ID,
      result: 'success',
      severity: 'info',
      request_id: REQUEST_ID,
      ip: '203.0.113.7',
      sort: 'occurred_at',
    }

    await renderPage()

    expect(mocks.useListAuditLogs).toHaveBeenCalledWith(
      {
        actor_id: ACTOR_ID,
        action: 'authn_login_success',
        target_type: 'user',
        target_id: TARGET_ID,
        result: 'success',
        severity: 'info',
        request_id: REQUEST_ID,
        ip: '203.0.113.7',
        from: WINDOW.from,
        to: WINDOW.to,
        page: 2,
        page_size: 50,
        sort: 'occurred_at',
      },
      expect.objectContaining({
        query: expect.objectContaining({
          placeholderData: expect.any(Function),
        }),
      })
    )
    expect(listOptions()?.query?.placeholderData).toBeTypeOf('function')
  })

  it('materializes the default 30-day preset as an absolute URL window', async () => {
    mocks.search = {}

    await renderPage()

    await vi.waitFor(() => {
      expect(mocks.navigate).toHaveBeenCalledTimes(1)
    })

    const search = searchFromCall()
    expect(search.range).toBe('30d')
    expect(search.from).toEqual(expect.any(String))
    expect(search.to).toEqual(expect.any(String))
    expect(
      Date.parse(search.to as string) - Date.parse(search.from as string)
    ).toBe(30 * 24 * 60 * 60 * 1000)
  })

  it('renders localized action labels with the raw code and hides request_id by default', async () => {
    const screen = await renderPage()

    await expect.element(screen.getByText('Signed in')).toBeInTheDocument()
    await expect
      .element(screen.getByText('authn_login_success'))
      .toBeInTheDocument()
    await expect.element(screen.getByText('alice')).toBeInTheDocument()
    await expect.element(screen.getByText('203.0.113.7')).toBeInTheDocument()
    await expect
      .element(screen.getByRole('button', { name: 'Copy actor ID' }))
      .toBeInTheDocument()
    await expect
      .element(screen.getByRole('button', { name: 'Copy target ID' }))
      .toBeInTheDocument()
    expect(screen.getByText(REQUEST_ID, { exact: true }).query()).toBeNull()
  })

  it('falls back to raw codes for unknown actions, targets, severities and results', async () => {
    mocks.useListAuditLogs.mockReturnValue({
      data: {
        ...LIST,
        items: [
          {
            ...SUMMARY,
            action: 'future_action',
            severity: 'verbose',
            result: 'unknown_result',
            target: { type: 'future_target', id: null, label: null },
          },
        ],
      },
      isPending: false,
      isFetching: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    })

    const screen = await renderPage()

    await expect.element(screen.getByText('future_action')).toBeInTheDocument()
    await expect.element(screen.getByText('verbose')).toBeInTheDocument()
    await expect.element(screen.getByText('unknown_result')).toBeInTheDocument()
    await expect.element(screen.getByText('future_target')).toBeInTheDocument()
  })

  it('resolves a time preset into an absolute URL window', async () => {
    const screen = await renderPage()

    await userEvent.click(screen.getByRole('combobox', { name: 'Time range' }))
    await userEvent.click(screen.getByRole('option', { name: 'Last 24 hours' }))

    const search = searchFromCall()
    expect(search.range).toBe('24h')
    expect(
      Date.parse(search.to as string) - Date.parse(search.from as string)
    ).toBe(24 * 60 * 60 * 1000)
  })

  it('rejects a custom range whose start is not before its end', async () => {
    const screen = await renderPage()

    await userEvent.click(screen.getByRole('combobox', { name: 'Time range' }))
    await userEvent.click(screen.getByRole('option', { name: 'Custom range…' }))

    await userEvent.fill(
      screen.getByRole('textbox', { name: 'From' }),
      '2026-11-10T12:00'
    )
    await userEvent.fill(
      screen.getByRole('textbox', { name: 'To' }),
      '2026-11-10T11:00'
    )
    await userEvent.click(screen.getByRole('button', { name: 'Apply' }))

    await expect
      .element(screen.getByRole('alert'))
      .toHaveTextContent('The start time must be before the end time.')
    expect(mocks.navigate).not.toHaveBeenCalled()
  })

  it('removes a single filter chip without clearing the window', async () => {
    mocks.search = { ...WINDOW, severity: 'warning' }
    const screen = await renderPage()

    await userEvent.click(
      screen.getByRole('button', { name: 'Remove filter Severity: Warning' })
    )

    expect(searchFromCall()).toMatchObject({
      severity: undefined,
      from: WINDOW.from,
      to: WINDOW.to,
    })
  })

  it('opens the detail sheet by pushing ?detail=<id>', async () => {
    const screen = await renderPage()

    await userEvent.click(screen.getByText('Signed in'))

    expect(mocks.navigate).toHaveBeenCalledTimes(1)
    const options = mocks.navigate.mock.calls[0][0] as {
      to: string
      replace?: boolean
    }
    expect(options.to).toBe('/audit-logs')
    expect(options.replace).not.toBe(true)
    expect(searchFromCall()).toEqual({ ...WINDOW, detail: LOG_ID })
  })

  it('keeps the absolute window while paging', async () => {
    mocks.useListAuditLogs.mockReturnValue({
      data: { ...LIST, total: 45 },
      isPending: false,
      isFetching: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    })

    const screen = await renderPage()

    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('common:table.goToNextPage'),
      })
    )

    const search = searchFromCall()
    expect(search).toMatchObject({
      ...WINDOW,
      page: 2,
      detail: undefined,
    })
  })

  it('re-reads the detail on every open and renders both views with masking', async () => {
    mocks.search = { ...WINDOW, detail: LOG_ID }
    mocks.useGetAuditLog.mockReturnValue({
      data: DETAIL,
      isPending: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    })

    const screen = await renderPage()

    expect(mocks.useGetAuditLog).toHaveBeenCalledWith(
      LOG_ID,
      expect.objectContaining({
        query: expect.objectContaining({
          enabled: true,
          staleTime: 0,
          gcTime: 0,
          refetchOnMount: 'always',
          refetchOnWindowFocus: false,
          refetchOnReconnect: false,
        }),
      })
    )

    await expect
      .element(screen.getByRole('heading', { name: 'Audit event' }))
      .toBeInTheDocument()
    await expect.element(screen.getByText('audit.read')).toBeInTheDocument()
    await expect
      .element(screen.getByRole('button', { name: 'Copy actor ID' }).first())
      .toBeInTheDocument()
    await expect
      .element(screen.getByRole('button', { name: 'Copy target ID' }).first())
      .toBeInTheDocument()

    await userEvent.click(screen.getByRole('tab', { name: 'Raw JSON' }))

    const json = screen.getByText(/"auth_method"/).element()
    expect(json.textContent).toContain('***')
    expect(json.textContent).not.toContain('leak-me')
    expect(json.textContent).not.toContain('also-secret')
  })

  it('shows an empty state when the detail has passed retention', async () => {
    mocks.search = { ...WINDOW, detail: LOG_ID }
    mocks.useGetAuditLog.mockReturnValue({
      data: undefined,
      isPending: false,
      isError: true,
      error: new ApiError({ code: 'not_found', status: 404 }),
      refetch: vi.fn(),
    })

    const screen = await renderPage()

    await expect
      .element(screen.getByText('Audit event unavailable'))
      .toBeInTheDocument()
  })

  it('filters by the detail actor and closes the sheet', async () => {
    mocks.search = { ...WINDOW, detail: LOG_ID }
    mocks.useGetAuditLog.mockReturnValue({
      data: DETAIL,
      isPending: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    })

    const screen = await renderPage()

    await userEvent.click(
      screen.getByRole('button', { name: "Filter this actor's other events" })
    )

    expect(searchFromCall()).toMatchObject({
      actor_id: ACTOR_ID,
      detail: undefined,
      page: undefined,
    })
  })

  it('maps list errors by code and offers a retry', async () => {
    mocks.useListAuditLogs.mockReturnValue({
      data: undefined,
      isPending: false,
      isFetching: false,
      isError: true,
      error: new ApiError({ code: 'invalid_time_range', status: 422 }),
      refetch: vi.fn(),
    })

    const screen = await renderPage()

    await expect
      .element(screen.getByText('The time range is invalid.'))
      .toBeInTheDocument()
    await expect
      .element(
        screen.getByRole('button', { name: i18n.t('common:error.retry') })
      )
      .toBeInTheDocument()
  })
})

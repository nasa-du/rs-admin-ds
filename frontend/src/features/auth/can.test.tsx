import { QueryClientProvider } from '@tanstack/react-query'
import { authWrapper, createAuthQueryClient, makeUser } from '@/test-utils/auth'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, renderHook } from 'vitest-browser-react'
import { authStore } from './auth-store'
import { Can } from './can'
import { useAuth, usePermission } from './use-auth'

const mocks = vi.hoisted(() => ({ getMe: vi.fn() }))

vi.mock('@/api/generated/rs-admin', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/api/generated/rs-admin')>()
  return { ...actual, getMe: mocks.getMe }
})

describe('Can', () => {
  it('renders children when the user holds the permission', async () => {
    const screen = await render(
      <Can permission='users.read'>
        <span>allowed</span>
      </Can>,
      { wrapper: authWrapper(makeUser({ permissions: ['users.read'] })) }
    )

    await expect.element(screen.getByText('allowed')).toBeInTheDocument()
  })

  it('renders nothing when the user lacks the permission', async () => {
    const screen = await render(
      <Can permission='users.read'>
        <span>allowed</span>
      </Can>,
      { wrapper: authWrapper(makeUser({ permissions: ['roles.read'] })) }
    )

    await expect.element(screen.getByText('allowed')).not.toBeInTheDocument()
  })

  it('always renders for a super admin, even without the permission', async () => {
    const screen = await render(
      <Can permission='audit.read'>
        <span>allowed</span>
      </Can>,
      {
        wrapper: authWrapper(
          makeUser({ is_super_admin: true, permissions: [] })
        ),
      }
    )

    await expect.element(screen.getByText('allowed')).toBeInTheDocument()
  })

  it('renders nothing when there is no current user', async () => {
    const screen = await render(
      <Can permission='users.read'>
        <span>allowed</span>
      </Can>,
      { wrapper: authWrapper(null) }
    )

    await expect.element(screen.getByText('allowed')).not.toBeInTheDocument()
  })
})

describe('usePermission', () => {
  it('reports the effective permission decision', async () => {
    const { result } = await renderHook(() => usePermission('roles.read'), {
      wrapper: authWrapper(makeUser({ permissions: ['roles.read'] })),
    })

    expect(result.current).toBe(true)
  })
})

describe('useAuth', () => {
  afterEach(() => {
    authStore.getState().reset()
    mocks.getMe.mockReset()
  })

  it("returns the cached current user from ['auth','me']", async () => {
    const user = makeUser({ username: 'alice' })
    const { result } = await renderHook(() => useAuth(), {
      wrapper: authWrapper(user),
    })

    expect(result.current).toEqual(user)
  })

  it('refetches through the generated API client when a session exists', async () => {
    authStore.getState().authenticate('access-token')
    const user = makeUser({ username: 'fetched' })
    mocks.getMe.mockResolvedValue(user)

    const queryClient = createAuthQueryClient(null)

    const { result } = await renderHook(() => useAuth(), {
      wrapper: ({ children }) => (
        <QueryClientProvider client={queryClient}>
          {children}
        </QueryClientProvider>
      ),
    })

    await vi.waitFor(() => expect(result.current).toEqual(user))
    expect(mocks.getMe).toHaveBeenCalledTimes(1)
  })

  it('returns null when no current user is cached', async () => {
    const { result } = await renderHook(() => useAuth(), {
      wrapper: authWrapper(null),
    })

    expect(result.current).toBeNull()
  })
})

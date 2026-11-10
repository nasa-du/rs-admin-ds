import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  AUTH_CHANNEL_NAME,
  broadcastSignedOut,
  subscribeToAuthBroadcast,
  type AuthChannel,
} from './auth-broadcast'
import { authStore } from './auth-store'
import {
  handleRemoteSignOut,
  performSignOut,
  signOutEverywhere,
} from './sign-out'

const api = vi.hoisted(() => ({
  logout: vi.fn(),
  logoutAll: vi.fn(),
}))

vi.mock('@/api/generated/rs-admin', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/api/generated/rs-admin')>()
  return { ...actual, logout: api.logout, logoutAll: api.logoutAll }
})

function fakeChannel() {
  const channel: AuthChannel & {
    posted: unknown[]
    closed: boolean
    emit: (data: unknown) => void
  } = {
    posted: [],
    closed: false,
    postMessage(message) {
      channel.posted.push(message)
    },
    close() {
      channel.closed = true
    },
    onmessage: null,
    emit(data) {
      channel.onmessage?.({ data })
    },
  }
  return channel
}

describe('auth broadcast', () => {
  it('uses the documented channel name', () => {
    expect(AUTH_CHANNEL_NAME).toBe('rs-admin-auth')
  })

  it('posts a signed-out message and closes the channel', () => {
    const channel = fakeChannel()

    broadcastSignedOut(() => channel)

    expect(channel.posted).toEqual([{ type: 'signed-out' }])
    expect(channel.closed).toBe(true)
  })

  it('ignores unrelated messages and unsubscribes cleanly', () => {
    const channel = fakeChannel()
    const handler = vi.fn()
    const unsubscribe = subscribeToAuthBroadcast(handler, () => channel)

    channel.emit({ type: 'other' })
    channel.emit(null)
    channel.emit({ type: 'signed-out' })

    expect(handler).toHaveBeenCalledTimes(1)
    expect(handler).toHaveBeenCalledWith({ type: 'signed-out' })

    unsubscribe()
    expect(channel.onmessage).toBeNull()
    expect(channel.closed).toBe(true)

    channel.emit({ type: 'signed-out' })
    expect(handler).toHaveBeenCalledTimes(1)
  })
})

describe('performSignOut', () => {
  it('clears local state and broadcasts after a successful server logout', async () => {
    const requestLogout = vi.fn(async () => undefined)
    const signOut = vi.fn()
    const broadcast = vi.fn()
    const notifyServerFailure = vi.fn()

    await expect(
      performSignOut({ requestLogout, signOut, broadcast, notifyServerFailure })
    ).resolves.toEqual({ ok: true })

    expect(requestLogout).toHaveBeenCalledTimes(1)
    expect(signOut).toHaveBeenCalledTimes(1)
    expect(broadcast).toHaveBeenCalledTimes(1)
    expect(notifyServerFailure).not.toHaveBeenCalled()
  })

  it('still clears local state and broadcasts when the server logout fails', async () => {
    const requestLogout = vi.fn(async () => {
      throw new Error('network down')
    })
    const signOut = vi.fn()
    const broadcast = vi.fn()
    const notifyServerFailure = vi.fn()

    await expect(
      performSignOut({ requestLogout, signOut, broadcast, notifyServerFailure })
    ).resolves.toEqual({ ok: false })

    expect(signOut).toHaveBeenCalledTimes(1)
    expect(broadcast).toHaveBeenCalledTimes(1)
    expect(notifyServerFailure).toHaveBeenCalledTimes(1)
  })
})

describe('signOutEverywhere', () => {
  beforeEach(() => {
    api.logoutAll.mockReset()
    api.logoutAll.mockResolvedValue(undefined)
    authStore.getState().reset()
  })

  it('revokes every server session and clears the local session', async () => {
    authStore.getState().authenticate('access-token')

    await expect(signOutEverywhere()).resolves.toEqual({ ok: true })

    expect(api.logoutAll).toHaveBeenCalledWith({})
    expect(authStore.getState().authStatus).toBe('unauthenticated')
    expect(authStore.getState().accessToken).toBeNull()
  })

  it('still clears the local session when the request fails', async () => {
    authStore.getState().authenticate('access-token')
    api.logoutAll.mockRejectedValue(new Error('network down'))

    await expect(signOutEverywhere()).resolves.toEqual({ ok: false })

    expect(authStore.getState().authStatus).toBe('unauthenticated')
  })
})

describe('handleRemoteSignOut', () => {
  it('clears state and leaves a public page where it is', () => {
    const signOut = vi.fn()
    const redirectToSignIn = vi.fn()

    handleRemoteSignOut({
      signOut,
      pathname: '/sign-in',
      href: '/sign-in',
      redirectToSignIn,
    })

    expect(signOut).toHaveBeenCalledTimes(1)
    expect(redirectToSignIn).not.toHaveBeenCalled()
  })

  it('redirects a protected page to sign-in with its path', () => {
    const signOut = vi.fn()
    const redirectToSignIn = vi.fn()

    handleRemoteSignOut({
      signOut,
      pathname: '/users',
      href: '/users?page=2',
      redirectToSignIn,
    })

    expect(signOut).toHaveBeenCalledTimes(1)
    expect(redirectToSignIn).toHaveBeenCalledWith('/users?page=2')
  })
})

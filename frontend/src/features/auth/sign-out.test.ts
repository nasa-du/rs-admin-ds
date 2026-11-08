import { describe, expect, it, vi } from 'vitest'
import {
  AUTH_CHANNEL_NAME,
  broadcastSignedOut,
  subscribeToAuthBroadcast,
  type AuthChannel,
} from './auth-broadcast'
import { handleRemoteSignOut, performSignOut } from './sign-out'

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

import { clearCookies } from '@/test-utils/cookies'
import { beforeEach, describe, expect, it } from 'vitest'
import { authStore } from './auth-store'

beforeEach(() => {
  authStore.getState().reset()
  localStorage.clear()
  sessionStorage.clear()
  clearCookies()
})

describe('auth store', () => {
  it('starts unknown without an access token', () => {
    expect(authStore.getState().accessToken).toBeNull()
    expect(authStore.getState().authStatus).toBe('unknown')
  })

  it('stores an authenticated session in memory', () => {
    authStore.getState().authenticate('access-token-1')

    expect(authStore.getState().accessToken).toBe('access-token-1')
    expect(authStore.getState().authStatus).toBe('authenticated')
  })

  it('clears the session to unauthenticated', () => {
    authStore.getState().authenticate('access-token-1')

    authStore.getState().clearSession()

    expect(authStore.getState().accessToken).toBeNull()
    expect(authStore.getState().authStatus).toBe('unauthenticated')
  })

  it('resets back to the cold-start state', () => {
    authStore.getState().authenticate('access-token-1')

    authStore.getState().reset()

    expect(authStore.getState().accessToken).toBeNull()
    expect(authStore.getState().authStatus).toBe('unknown')
  })

  it('never writes the access token to browser storage or cookies', () => {
    const cookieBefore = document.cookie

    authStore.getState().authenticate('secret-access-token')

    expect(localStorage.length).toBe(0)
    expect(sessionStorage.length).toBe(0)
    expect(document.cookie).toBe(cookieBefore)

    const labels = [
      ...Object.keys(localStorage),
      ...Object.keys(sessionStorage),
    ].join(',')
    expect(labels).not.toContain('secret-access-token')
  })
})

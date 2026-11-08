import { describe, expect, it, vi } from 'vitest'
import {
  handlePasswordChangeRequired,
  notifyPasswordChangeRequired,
  onPasswordChangeRequired,
} from './auth-interruptions'

describe('password-change interruption', () => {
  it('notifies subscribers until they unsubscribe', () => {
    const listener = vi.fn()
    const unsubscribe = onPasswordChangeRequired(listener)

    notifyPasswordChangeRequired()
    expect(listener).toHaveBeenCalledTimes(1)

    unsubscribe()
    notifyPasswordChangeRequired()
    expect(listener).toHaveBeenCalledTimes(1)
  })

  it('redirects with the current path', () => {
    const redirectToChangePassword = vi.fn()

    handlePasswordChangeRequired({
      pathname: '/users',
      href: '/users?page=2',
      redirectToChangePassword,
    })

    expect(redirectToChangePassword).toHaveBeenCalledWith('/users?page=2')
  })

  it('does not loop when already on /change-password', () => {
    const redirectToChangePassword = vi.fn()

    handlePasswordChangeRequired({
      pathname: '/change-password/',
      href: '/change-password?redirect=/users',
      redirectToChangePassword,
    })

    expect(redirectToChangePassword).not.toHaveBeenCalled()
  })
})

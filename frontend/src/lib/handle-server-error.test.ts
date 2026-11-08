import i18n from '@/i18n'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError } from './api-error'
import { handleServerError } from './handle-server-error'

const toastError = vi.hoisted(() => vi.fn())

vi.mock('sonner', () => ({
  toast: {
    error: toastError,
  },
}))

beforeEach(async () => {
  vi.mocked(toastError).mockClear()
  await i18n.changeLanguage('en-US')
})

describe('handleServerError', () => {
  it('shows a generic localized message when the error is not recognised', () => {
    handleServerError(new Error('network'))

    expect(toastError).toHaveBeenCalledWith(
      'An unexpected error occurred. Please try again later.'
    )
  })

  it('maps a ProblemDetails code to a localized message', () => {
    handleServerError({ code: 'invalid_credentials', status: 401 })

    expect(toastError).toHaveBeenCalledWith(
      'The username or password is incorrect.'
    )
  })

  it('maps an axios-style response ProblemDetails', () => {
    handleServerError({
      response: {
        status: 403,
        data: { code: 'role_immutable', status: 403 },
      },
    })

    expect(toastError).toHaveBeenCalledWith(
      'Built-in roles cannot be modified.'
    )
  })

  it('does not repeat a prompt already surfaced by the API client', () => {
    const error = new ApiError({ code: 'account_disabled', status: 403 })
    error.handled = true

    handleServerError(error)

    expect(toastError).not.toHaveBeenCalled()
  })

  it('falls back to the unexpected message for unknown codes', () => {
    handleServerError({ code: 'future_code_not_in_catalog', status: 400 })

    expect(toastError).toHaveBeenCalledWith(
      'An unexpected error occurred. Please try again later.'
    )
  })

  it('maps a TypeError to the network error message', () => {
    handleServerError(new TypeError('Failed to fetch'))

    expect(toastError).toHaveBeenCalledWith(
      'The network request failed. Check your connection and try again.'
    )
  })

  it('uses the zh-CN resource after switching language', async () => {
    await i18n.changeLanguage('zh-CN')

    handleServerError({ code: 'invalid_credentials', status: 401 })

    expect(toastError).toHaveBeenCalledWith('用户名或密码不正确。')
  })

  it('logs the error to the console in development', () => {
    const log = vi.spyOn(console, 'log').mockImplementation(() => {})
    const err = new Error('logged')

    handleServerError(err)

    expect(log).toHaveBeenCalledTimes(1)
    expect(log).toHaveBeenCalledWith(err)

    log.mockRestore()
  })

  it('does not log the error to the console in production', () => {
    vi.stubEnv('DEV', false)

    const log = vi.spyOn(console, 'log').mockImplementation(() => {})
    const err = new Error('not logged')

    handleServerError(err)

    expect(log).not.toHaveBeenCalled()

    log.mockRestore()
  })
})

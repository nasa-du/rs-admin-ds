import {
  createReauthTestController as setup,
  reauthRequired,
} from '@/test-utils/reauth'
import { describe, expect, it, vi } from 'vitest'
import { ApiError } from '@/lib/api-error'
import {
  REAUTH_RACE_WINDOW_MS,
  ReauthCancelledError,
} from './reauth-controller'

function apiError(status: number, code: string) {
  return new ApiError({ code, status })
}

describe('runWithReauth', () => {
  it('prompts, applies the new access token, and retries the operation once', async () => {
    const { controller, deps } = setup()
    const operation = vi
      .fn()
      .mockRejectedValueOnce(reauthRequired())
      .mockResolvedValueOnce('saved')

    const result = controller.runWithReauth(operation)

    await vi.waitFor(() => expect(controller.getSnapshot().open).toBe(true))
    expect(deps.reauthenticate).not.toHaveBeenCalled()

    await controller.submitPassword('current-password')

    await expect(result).resolves.toBe('saved')
    expect(deps.reauthenticate).toHaveBeenCalledTimes(1)
    expect(deps.reauthenticate).toHaveBeenCalledWith('current-password')
    expect(deps.applyAccessToken).toHaveBeenCalledWith('fresh-token')
    expect(operation).toHaveBeenCalledTimes(2)
    expect(controller.getSnapshot()).toEqual({
      open: false,
      submitting: false,
      error: null,
    })
  })

  it('shares one prompt and one reauthentication between concurrent callers', async () => {
    const { controller, deps } = setup()
    const first = vi
      .fn()
      .mockRejectedValueOnce(reauthRequired())
      .mockResolvedValueOnce('first')
    const second = vi
      .fn()
      .mockRejectedValueOnce(reauthRequired())
      .mockResolvedValueOnce('second')

    const firstResult = controller.runWithReauth(first)
    const secondResult = controller.runWithReauth(second)

    await vi.waitFor(() => expect(controller.getSnapshot().open).toBe(true))
    await controller.submitPassword('current-password')

    await expect(firstResult).resolves.toBe('first')
    await expect(secondResult).resolves.toBe('second')
    expect(deps.reauthenticate).toHaveBeenCalledTimes(1)
    expect(first).toHaveBeenCalledTimes(2)
    expect(second).toHaveBeenCalledTimes(2)
  })

  it('rejects with ReauthCancelledError and does not execute the operation again', async () => {
    const { controller, deps } = setup()
    const operation = vi.fn().mockRejectedValueOnce(reauthRequired())

    const result = controller.runWithReauth(operation)

    await vi.waitFor(() => expect(controller.getSnapshot().open).toBe(true))
    controller.cancel()

    await expect(result).rejects.toBeInstanceOf(ReauthCancelledError)
    expect(operation).toHaveBeenCalledTimes(1)
    expect(deps.reauthenticate).not.toHaveBeenCalled()
    expect(controller.getSnapshot().open).toBe(false)
  })

  it('propagates a second reauth_required instead of looping', async () => {
    const { controller } = setup()
    const operation = vi.fn().mockRejectedValue(reauthRequired())

    const result = controller.runWithReauth(operation)
    await vi.waitFor(() => expect(controller.getSnapshot().open).toBe(true))
    await controller.submitPassword('current-password')

    await expect(result).rejects.toMatchObject({ code: 'reauth_required' })
    expect(operation).toHaveBeenCalledTimes(2)
  })

  it('retries without prompting when a successful reauth is under 5 seconds old', async () => {
    const { controller, deps, advanceTime } = setup()
    const first = vi
      .fn()
      .mockRejectedValueOnce(reauthRequired())
      .mockResolvedValueOnce('first')

    const firstResult = controller.runWithReauth(first)
    await vi.waitFor(() => expect(controller.getSnapshot().open).toBe(true))
    await controller.submitPassword('current-password')
    await expect(firstResult).resolves.toBe('first')

    advanceTime(REAUTH_RACE_WINDOW_MS - 1)
    const second = vi
      .fn()
      .mockRejectedValueOnce(reauthRequired())
      .mockResolvedValueOnce('second')

    await expect(controller.runWithReauth(second)).resolves.toBe('second')
    expect(second).toHaveBeenCalledTimes(2)
    expect(deps.reauthenticate).toHaveBeenCalledTimes(1)
    expect(controller.getSnapshot().open).toBe(false)
  })

  it('prompts again when the last successful reauth is at least 5 seconds old', async () => {
    const { controller, deps, advanceTime } = setup()
    const first = vi
      .fn()
      .mockRejectedValueOnce(reauthRequired())
      .mockResolvedValueOnce('first')

    const firstResult = controller.runWithReauth(first)
    await vi.waitFor(() => expect(controller.getSnapshot().open).toBe(true))
    await controller.submitPassword('current-password')
    await expect(firstResult).resolves.toBe('first')

    advanceTime(REAUTH_RACE_WINDOW_MS)
    const second = vi
      .fn()
      .mockRejectedValueOnce(reauthRequired())
      .mockResolvedValueOnce('second')

    const secondResult = controller.runWithReauth(second)
    await vi.waitFor(() => expect(controller.getSnapshot().open).toBe(true))
    expect(deps.reauthenticate).toHaveBeenCalledTimes(1)

    await controller.submitPassword('current-password')
    await expect(secondResult).resolves.toBe('second')
    expect(deps.reauthenticate).toHaveBeenCalledTimes(2)
  })

  it('does not touch the prompt for unrelated errors', async () => {
    const { controller, deps } = setup()
    const failure = apiError(409, 'role_in_use')

    await expect(
      controller.runWithReauth(() => Promise.reject(failure))
    ).rejects.toBe(failure)
    expect(controller.getSnapshot().open).toBe(false)
    expect(deps.reauthenticate).not.toHaveBeenCalled()
  })

  it('stays closed when cancelled while the password request is in flight', async () => {
    const { controller, deps } = setup()
    let rejectRequest: (error: unknown) => void = () => undefined
    deps.reauthenticate.mockImplementationOnce(
      () =>
        new Promise((_resolve, reject) => {
          rejectRequest = reject
        })
    )
    const operation = vi.fn().mockRejectedValueOnce(reauthRequired())

    const result = controller.runWithReauth(operation)
    await vi.waitFor(() => expect(controller.getSnapshot().open).toBe(true))

    const submission = controller.submitPassword('current-password')
    controller.cancel()
    rejectRequest(apiError(422, 'invalid_current_password'))

    await expect(submission).resolves.toBeUndefined()
    await expect(result).rejects.toBeInstanceOf(ReauthCancelledError)
    expect(controller.getSnapshot().open).toBe(false)
    expect(operation).toHaveBeenCalledTimes(1)
  })
})

describe('password submission', () => {
  it('keeps the dialog open with the error when the password is wrong', async () => {
    const { controller, deps } = setup()
    const invalidPassword = apiError(422, 'invalid_current_password')
    deps.reauthenticate.mockRejectedValueOnce(invalidPassword)
    const operation = vi
      .fn()
      .mockRejectedValueOnce(reauthRequired())
      .mockResolvedValueOnce('saved')

    const result = controller.runWithReauth(operation)
    await vi.waitFor(() => expect(controller.getSnapshot().open).toBe(true))

    await expect(controller.submitPassword('wrong-password')).rejects.toBe(
      invalidPassword
    )
    expect(controller.getSnapshot()).toMatchObject({
      open: true,
      submitting: false,
      error: invalidPassword,
    })
    expect(operation).toHaveBeenCalledTimes(1)

    await controller.submitPassword('right-password')
    await expect(result).resolves.toBe('saved')
    expect(operation).toHaveBeenCalledTimes(2)
    expect(controller.getSnapshot()).toEqual({
      open: false,
      submitting: false,
      error: null,
    })
  })

  it('closes the dialog and rejects when the session must change its password', async () => {
    const { controller, deps } = setup()
    const passwordChangeRequired = apiError(403, 'password_change_required')
    deps.reauthenticate.mockRejectedValueOnce(passwordChangeRequired)
    const operation = vi.fn().mockRejectedValueOnce(reauthRequired())

    const result = controller.runWithReauth(operation)
    await vi.waitFor(() => expect(controller.getSnapshot().open).toBe(true))
    await controller.submitPassword('current-password')

    await expect(result).rejects.toBe(passwordChangeRequired)
    expect(controller.getSnapshot().open).toBe(false)
    expect(operation).toHaveBeenCalledTimes(1)
  })
})

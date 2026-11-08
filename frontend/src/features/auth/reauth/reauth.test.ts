import { reauthRequired } from '@/test-utils/reauth'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from '@/api/mutator'
import { authStore } from '@/features/auth/auth-store'
import { reauthController, runWithReauth } from './reauth'

afterEach(() => {
  reauthController.cancel()
  authStore.getState().reset()
  vi.restoreAllMocks()
})

describe('application reauth controller', () => {
  it('calls POST /auth/reauthenticate and replaces the in-memory access token', async () => {
    authStore.getState().authenticate('stale-token')
    const request = vi.spyOn(apiClient, 'request').mockResolvedValue({
      data: {
        access_token: 'fresh-token',
        auth_time: 1,
        expires_in: 900,
        token_type: 'Bearer',
      },
    } as never)
    const operation = vi
      .fn()
      .mockRejectedValueOnce(reauthRequired())
      .mockResolvedValueOnce('saved')

    const result = runWithReauth(operation)

    await vi.waitFor(() =>
      expect(reauthController.getSnapshot().open).toBe(true)
    )
    await reauthController.submitPassword('current-password')

    await expect(result).resolves.toBe('saved')
    expect(request).toHaveBeenCalledWith(
      expect.objectContaining({
        url: '/api/v1/auth/reauthenticate',
        method: 'POST',
        data: { password: 'current-password' },
      })
    )
    expect(authStore.getState().accessToken).toBe('fresh-token')
    expect(operation).toHaveBeenCalledTimes(2)
  })
})

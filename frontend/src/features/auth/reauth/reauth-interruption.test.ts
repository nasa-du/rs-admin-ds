import {
  AxiosError,
  type AxiosResponse,
  type InternalAxiosRequestConfig,
} from 'axios'
import { reauthRequired } from '@/test-utils/reauth'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from '@/api/mutator'
import { onPasswordChangeRequired } from '@/features/auth/auth-interruptions'
import { authStore } from '@/features/auth/auth-store'
import { reauthController, runWithReauth } from './reauth'

function problemResponse(
  config: InternalAxiosRequestConfig,
  status: number,
  data: unknown
) {
  const response: AxiosResponse = {
    data,
    status,
    statusText: '',
    headers: {},
    config,
  }
  return new AxiosError(
    `Request failed with status ${status}`,
    'ERR_BAD_RESPONSE',
    config,
    {},
    response
  )
}

afterEach(() => {
  reauthController.cancel()
  authStore.getState().reset()
  vi.restoreAllMocks()
})

describe('reauth password-change interruption', () => {
  it('closes the prompt and notifies the shell when the session must change its password', async () => {
    authStore.getState().authenticate('access-token')
    const originalAdapter = apiClient.defaults.adapter
    const passwordChangeListener = vi.fn()
    const unsubscribe = onPasswordChangeRequired(passwordChangeListener)

    apiClient.defaults.adapter = async (config) => {
      throw problemResponse(config as InternalAxiosRequestConfig, 403, {
        code: 'password_change_required',
        status: 403,
        request_id: 'req-reauth-1',
      })
    }

    try {
      const operation = vi.fn().mockRejectedValueOnce(reauthRequired())
      const result = runWithReauth(operation)

      await vi.waitFor(() =>
        expect(reauthController.getSnapshot().open).toBe(true)
      )
      await reauthController.submitPassword('current-password')

      await expect(result).rejects.toMatchObject({
        code: 'password_change_required',
      })
      expect(passwordChangeListener).toHaveBeenCalledTimes(1)
      expect(reauthController.getSnapshot().open).toBe(false)
      expect(operation).toHaveBeenCalledTimes(1)
    } finally {
      apiClient.defaults.adapter = originalAdapter
      unsubscribe()
    }
  })
})

import { vi } from 'vitest'
import { ApiError } from '@/lib/api-error'
import { createReauthController } from '@/features/auth/reauth/reauth-controller'

/** A reauthentication refusal as the API client throws it. */
export function reauthRequired() {
  return new ApiError({ code: 'reauth_required', status: 403 })
}

/**
 * Controller fixture shared by the reauth unit and dialog tests. `now` is
 * controllable so the 5-second race window can be exercised deterministically.
 */
export function createReauthTestController() {
  let now = 1_000_000
  const deps = {
    reauthenticate: vi.fn().mockResolvedValue({ access_token: 'fresh-token' }),
    applyAccessToken: vi.fn(),
    now: () => now,
  }
  const controller = createReauthController(deps)

  return {
    controller,
    deps,
    advanceTime: (ms: number) => {
      now += ms
    },
  }
}

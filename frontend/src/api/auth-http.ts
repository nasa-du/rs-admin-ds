import axios from 'axios'
import { apiBaseUrl } from './base-url'

/**
 * Raw transport for the auth bootstrap (`refresh` → `/me`).
 *
 * It intentionally has no auth interceptors: a 401 from these calls is a
 * classification result for `bootstrap()`, not a reason to start another
 * refresh. Business calls go through `apiClient` in `mutator.ts`.
 */
export const authHttp = axios.create({
  baseURL: apiBaseUrl,
  headers: { Accept: 'application/json' },
})

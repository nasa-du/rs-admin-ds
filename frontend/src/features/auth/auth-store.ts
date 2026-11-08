import { create } from 'zustand'

/**
 * Whether the client currently knows who the user is.
 *
 * - `unknown`: cold start before bootstrap, or a retryable bootstrap failure
 *   (network/5xx). The access token is not trusted yet.
 * - `authenticated`: a usable in-memory access token exists.
 * - `unauthenticated`: no session; the user must sign in.
 */
export type AuthStatus = 'unknown' | 'authenticated' | 'unauthenticated'

export type AuthState = {
  accessToken: string | null
  authStatus: AuthStatus
  authenticate: (accessToken: string) => void
  clearSession: () => void
  reset: () => void
}

/**
 * Auth state deliberately lives only in memory (ADR-0006 §1): no `persist`
 * middleware, no localStorage/sessionStorage, no JS-readable cookie. The
 * refresh token stays in its HttpOnly `__Host-refresh` cookie.
 */
export const useAuthStore = create<AuthState>()((set) => ({
  accessToken: null,
  authStatus: 'unknown',
  authenticate: (accessToken) =>
    set({ accessToken, authStatus: 'authenticated' }),
  clearSession: () => set({ accessToken: null, authStatus: 'unauthenticated' }),
  reset: () => set({ accessToken: null, authStatus: 'unknown' }),
}))

export const authStore = useAuthStore

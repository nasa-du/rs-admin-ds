import { reauthenticate as reauthenticateApi } from '@/api/generated/rs-admin'
import { authSession } from '@/features/auth/auth-session'
import {
  type ReauthController,
  createReauthController,
} from './reauth-controller'

/**
 * Application-wide reauthentication controller. The dialog renders from this
 * instance and feature mutations call `runWithReauth`, so every 403
 * `reauth_required` around the app shares one prompt.
 */
export const reauthController = createReauthController({
  reauthenticate: (password) => reauthenticateApi({ password }),
  applyAccessToken: (accessToken) => authSession.applyAccessToken(accessToken),
})

/** Wrap a reauthentication-gated mutation (ADR-0006 §7). */
export const runWithReauth: ReauthController['runWithReauth'] =
  reauthController.runWithReauth

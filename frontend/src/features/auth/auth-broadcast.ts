/**
 * Cross-tab sign-out sync (ADR-0006 §5). Only sign-out is broadcast; access
 * tokens are never shared between tabs.
 */

export const AUTH_CHANNEL_NAME = 'rs-admin-auth'

export type AuthBroadcastMessage = { type: 'signed-out' }

export type AuthChannel = {
  postMessage: (message: AuthBroadcastMessage) => void
  close: () => void
  onmessage: ((event: { data: unknown }) => void) | null
}

export type AuthChannelFactory = () => AuthChannel | null

function createBrowserChannel(): AuthChannel | null {
  if (typeof BroadcastChannel === 'undefined') return null
  return new BroadcastChannel(AUTH_CHANNEL_NAME) as unknown as AuthChannel
}

function isSignedOutMessage(data: unknown): data is AuthBroadcastMessage {
  return (
    typeof data === 'object' &&
    data !== null &&
    (data as { type?: unknown }).type === 'signed-out'
  )
}

export function broadcastSignedOut(
  createChannel: AuthChannelFactory = createBrowserChannel
): void {
  const channel = createChannel()
  if (!channel) return
  channel.postMessage({ type: 'signed-out' })
  channel.close()
}

/** Subscribe to cross-tab sign-out; returns an unsubscribe function. */
export function subscribeToAuthBroadcast(
  handler: (message: AuthBroadcastMessage) => void,
  createChannel: AuthChannelFactory = createBrowserChannel
): () => void {
  const channel = createChannel()
  if (!channel) return () => undefined

  channel.onmessage = (event) => {
    if (isSignedOutMessage(event.data)) handler({ type: 'signed-out' })
  }

  return () => {
    channel.onmessage = null
    channel.close()
  }
}

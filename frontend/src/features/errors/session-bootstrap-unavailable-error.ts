/** Thrown by route guards when bootstrap failed transiently (network/5xx). */
export class SessionBootstrapUnavailableError extends Error {
  constructor() {
    super('Session bootstrap is unavailable')
    this.name = 'SessionBootstrapUnavailableError'
  }
}

/**
 * Error types shared by the API client and the i18n error mapper.
 *
 * The backend returns RFC 7807 ProblemDetails with a stable `code`. The
 * frontend never renders `title`/`detail` directly; it maps `code` to a
 * localized message.
 */

export type FieldError = {
  pointer: string
  code: string
  params?: Record<string, unknown>
}

export type ProblemDetails = {
  type?: string
  code: string
  title?: string
  status?: number
  detail?: string
  instance?: string
  request_id?: string
  errors?: FieldError[]
}

export type ApiErrorOptions = {
  retryAfterSeconds?: number
}

export class ApiError extends Error {
  readonly problem: ProblemDetails
  readonly retryAfterSeconds: number | null
  /**
   * Set once the error has produced a user-visible prompt. Global error
   * handlers use it to avoid repeating the same toast (ADR-0006 §3).
   */
  handled = false

  constructor(problem: ProblemDetails, options: ApiErrorOptions = {}) {
    super(problem.title ?? problem.detail ?? problem.code)
    this.name = 'ApiError'
    this.problem = problem
    this.retryAfterSeconds = options.retryAfterSeconds ?? null
  }

  get code(): string {
    return this.problem.code
  }

  get status(): number | undefined {
    return this.problem.status
  }

  get requestId(): string | undefined {
    return this.problem.request_id
  }

  get fieldErrors(): FieldError[] {
    return this.problem.errors ?? []
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isProblemDetails(value: unknown): value is ProblemDetails {
  return isRecord(value) && typeof value.code === 'string'
}

/**
 * Normalize the error shapes we expect to see in the app:
 * - our own `ApiError`
 * - a raw ProblemDetails object
 * - an axios-style error with `response.data` ProblemDetails
 */
/**
 * Mark an error as already surfaced so global handlers skip the duplicate
 * toast (ADR-0006 §3).
 */
export function markApiErrorHandled(error: unknown): void {
  const normalized = toApiError(error)
  if (normalized instanceof ApiError) normalized.handled = true
}

export function toApiError(error: unknown): ApiError | null {
  if (error instanceof ApiError) return error

  // Axios errors also carry a string `code` (e.g. `ERR_NETWORK`), so check the
  // response shape before the raw ProblemDetails shape.
  if (isRecord(error) && isRecord(error.response)) {
    return fromResponse(error.response)
  }

  if (isRecord(error) && error.isAxiosError === true) {
    if (error.code === 'ERR_CANCELED') return null
    return new ApiError({
      code: 'network_error',
      title: typeof error.message === 'string' ? error.message : undefined,
    })
  }

  if (isProblemDetails(error)) {
    return new ApiError(error)
  }

  return null
}

function fromResponse(response: Record<string, unknown>): ApiError | null {
  const retryAfter = getRetryAfterHeader(response.headers)
  const requestId = getHeader(response.headers, 'x-request-id')

  if (isProblemDetails(response.data)) {
    return new ApiError(
      {
        ...response.data,
        request_id:
          response.data.request_id ??
          (typeof requestId === 'string' ? requestId : undefined),
      },
      { retryAfterSeconds: retryAfter }
    )
  }

  const status = Number(response.status)
  if (!Number.isInteger(status) || status < 400) return null

  return new ApiError(
    {
      code: fallbackCodeForStatus(status),
      status,
      title:
        typeof response.statusText === 'string'
          ? response.statusText
          : undefined,
      request_id: typeof requestId === 'string' ? requestId : undefined,
    },
    { retryAfterSeconds: retryAfter }
  )
}

/**
 * Status-code fallback for responses that are not ProblemDetails (gateway
 * HTML, proxy errors). Codes are stable catalog values so the i18n mapper
 * still produces a localized message.
 */
function fallbackCodeForStatus(status: number): string {
  switch (status) {
    case 400:
    case 409:
    case 422:
      return 'validation_failed'
    case 401:
      return 'access_token_invalid'
    case 403:
      return 'forbidden'
    case 404:
      return 'not_found'
    case 405:
      return 'method_not_allowed'
    case 413:
      return 'payload_too_large'
    case 415:
      return 'unsupported_media_type'
    case 429:
      return 'rate_limited'
    case 500:
      return 'internal_error'
    default:
      return status >= 500 ? 'service_unavailable' : 'validation_failed'
  }
}

function getHeader(
  headers: unknown,
  name: string
): string | number | undefined {
  if (!isRecord(headers)) return undefined

  const lowerName = name.toLowerCase()
  for (const [key, value] of Object.entries(headers)) {
    if (key.toLowerCase() !== lowerName) continue
    if (typeof value === 'string' || typeof value === 'number') return value
  }
  return undefined
}

function clampRetryAfter(value: unknown): number | null {
  const parsed =
    typeof value === 'number'
      ? value
      : typeof value === 'string' && value.trim() !== ''
        ? Number(value)
        : Number.NaN

  if (!Number.isFinite(parsed)) return null
  return Math.max(0, Math.trunc(parsed))
}

function getRetryAfterHeader(headers: unknown): number | undefined {
  return clampRetryAfter(getHeader(headers, 'retry-after')) ?? undefined
}

const DEFAULT_RETRY_AFTER_SECONDS = 60

/**
 * Returns the server's `Retry-After` hint in seconds. For a 429 without a
 * usable hint, falls back to 60 seconds (ADR-0007 §6).
 */
export function getRetryAfterSeconds(
  error: unknown,
  fallbackSeconds = DEFAULT_RETRY_AFTER_SECONDS
): number | null {
  const apiError = toApiError(error)
  if (apiError?.retryAfterSeconds != null) return apiError.retryAfterSeconds

  if (isRecord(error)) {
    const direct = clampRetryAfter(error.retryAfterSeconds)
    if (direct != null) return direct

    if (isRecord(error.response)) {
      const fromHeader = clampRetryAfter(
        getHeader(error.response.headers, 'retry-after')
      )
      if (fromHeader != null) return fromHeader
    }
  }

  const responseStatus =
    isRecord(error) && isRecord(error.response)
      ? Number(error.response.status)
      : undefined
  const isRateLimited =
    apiError?.status === 429 ||
    (isRecord(error) && Number(error.status) === 429) ||
    responseStatus === 429
  if (isRateLimited) {
    return clampRetryAfter(fallbackSeconds) ?? DEFAULT_RETRY_AFTER_SECONDS
  }

  return null
}

export function getRequestId(error: unknown): string | undefined {
  const apiError = toApiError(error)
  if (apiError?.requestId) return apiError.requestId

  if (isRecord(error)) {
    if (typeof error.request_id === 'string') return error.request_id
    if (isRecord(error.response)) {
      const header = getHeader(error.response.headers, 'x-request-id')
      if (typeof header === 'string') return header
    }
  }

  return undefined
}

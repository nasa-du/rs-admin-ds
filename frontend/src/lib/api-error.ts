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
export function toApiError(error: unknown): ApiError | null {
  if (error instanceof ApiError) return error

  if (isProblemDetails(error)) {
    return new ApiError(error)
  }

  if (
    isRecord(error) &&
    isRecord(error.response) &&
    isProblemDetails(error.response.data)
  ) {
    const retryAfter = getRetryAfterHeader(error.response.headers)
    return new ApiError(error.response.data, { retryAfterSeconds: retryAfter })
  }

  return null
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

export function getRetryAfterSeconds(error: unknown): number | null {
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

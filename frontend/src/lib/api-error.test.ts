import { describe, expect, it } from 'vitest'
import {
  ApiError,
  getRequestId,
  getRetryAfterSeconds,
  toApiError,
} from './api-error'

describe('api-error', () => {
  it('normalizes a raw ProblemDetails object', () => {
    const error = toApiError({
      code: 'invalid_credentials',
      status: 401,
      request_id: 'req-1',
    })

    expect(error).toBeInstanceOf(ApiError)
    expect(error?.code).toBe('invalid_credentials')
    expect(error?.status).toBe(401)
    expect(error?.requestId).toBe('req-1')
  })

  it('normalizes an axios-style response', () => {
    const error = toApiError({
      response: {
        status: 429,
        data: { code: 'rate_limited', status: 429, request_id: 'req-2' },
        headers: { 'Retry-After': '12' },
      },
    })

    expect(error?.code).toBe('rate_limited')
    expect(error?.retryAfterSeconds).toBe(12)
    expect(getRetryAfterSeconds(error)).toBe(12)
  })

  it('returns null for unknown error shapes', () => {
    expect(toApiError(new Error('boom'))).toBeNull()
    expect(toApiError(undefined)).toBeNull()
  })

  it('falls back to a status-code error for non-ProblemDetails responses', () => {
    const error = toApiError({
      response: {
        status: 502,
        statusText: 'Bad Gateway',
        data: '<html>bad gateway</html>',
        headers: { 'x-request-id': 'req-gateway' },
      },
    })

    expect(error).toBeInstanceOf(ApiError)
    expect(error?.status).toBe(502)
    expect(error?.code).toBe('service_unavailable')
    expect(error?.requestId).toBe('req-gateway')
  })

  it('maps common fallback statuses to stable error codes', () => {
    const codeFor = (status: number) =>
      toApiError({ response: { status, data: 'not problem details' } })?.code

    expect(codeFor(401)).toBe('access_token_invalid')
    expect(codeFor(403)).toBe('forbidden')
    expect(codeFor(404)).toBe('not_found')
    expect(codeFor(429)).toBe('rate_limited')
    expect(codeFor(500)).toBe('internal_error')
    expect(codeFor(503)).toBe('service_unavailable')
  })

  it('keeps Retry-After on a non-ProblemDetails 429', () => {
    const error = toApiError({
      response: {
        status: 429,
        data: 'slow down',
        headers: { 'Retry-After': '30' },
      },
    })

    expect(error?.code).toBe('rate_limited')
    expect(error?.retryAfterSeconds).toBe(30)
  })

  it('prefers ProblemDetails over the status fallback', () => {
    const error = toApiError({
      response: {
        status: 500,
        data: { code: 'internal_error', status: 500, detail: 'db down' },
        headers: {},
      },
    })

    expect(error?.code).toBe('internal_error')
    expect(error?.problem.detail).toBe('db down')
  })

  it('clamps Retry-After to a non-negative integer', () => {
    expect(
      getRetryAfterSeconds({ response: { headers: { 'retry-after': '-5' } } })
    ).toBe(0)
    expect(
      getRetryAfterSeconds({
        response: { headers: { 'retry-after': '3.9' } },
      })
    ).toBe(3)
  })

  it('ignores invalid Retry-After values without a 429 status', () => {
    expect(
      getRetryAfterSeconds({ response: { headers: { 'retry-after': 'soon' } } })
    ).toBeNull()
    expect(getRetryAfterSeconds(new Error('boom'))).toBeNull()
  })

  it('falls back to 60 seconds for a 429 without a usable header', () => {
    expect(getRetryAfterSeconds({ code: 'rate_limited', status: 429 })).toBe(60)
    expect(
      getRetryAfterSeconds({
        response: { status: 429, headers: { 'retry-after': 'soon' } },
      })
    ).toBe(60)
  })

  it('accepts a custom retry fallback', () => {
    expect(
      getRetryAfterSeconds({ code: 'rate_limited', status: 429 }, 15)
    ).toBe(15)
  })

  it('reads request ids from ProblemDetails or the X-Request-Id header', () => {
    expect(getRequestId({ code: 'not_found', request_id: 'req-3' })).toBe(
      'req-3'
    )
    expect(
      getRequestId({
        response: { headers: { 'x-request-id': 'req-4' } },
      })
    ).toBe('req-4')
    expect(getRequestId(new Error('boom'))).toBeUndefined()
  })
})

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

  it('ignores invalid Retry-After values', () => {
    expect(
      getRetryAfterSeconds({ response: { headers: { 'retry-after': 'soon' } } })
    ).toBeNull()
    expect(getRetryAfterSeconds(new Error('boom'))).toBeNull()
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

import { act } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { ApiError } from '@/lib/api-error'
import { useRetryAfter } from './use-retry-after'

// vitest-browser-react toggles this flag around its own `act` calls, so the
// direct advances below must set it before invoking React's `act`.
function advance(ms: number) {
  ;(
    globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true
  act(() => vi.advanceTimersByTime(ms))
}

function Probe({ error }: { error: unknown }) {
  const remaining = useRetryAfter(error)
  return <span data-testid='remaining'>{remaining}</span>
}

function rateLimited(retryAfterSeconds: number) {
  return new ApiError(
    { code: 'rate_limited', status: 429 },
    { retryAfterSeconds }
  )
}

afterEach(() => {
  vi.useRealTimers()
})

describe('useRetryAfter', () => {
  it('starts from the Retry-After hint and counts down to zero', async () => {
    vi.useFakeTimers()
    const screen = await render(<Probe error={rateLimited(3)} />)

    const remaining = screen.getByTestId('remaining')
    await expect.element(remaining).toHaveTextContent('3')

    advance(1000)
    await expect.element(remaining).toHaveTextContent('2')

    advance(2000)
    await expect.element(remaining).toHaveTextContent('0')

    advance(2000)
    await expect.element(remaining).toHaveTextContent('0')
  })

  it('restarts when a new 429 error arrives', async () => {
    vi.useFakeTimers()
    const screen = await render(<Probe error={rateLimited(2)} />)
    advance(2000)
    await expect.element(screen.getByTestId('remaining')).toHaveTextContent('0')

    await screen.rerender(<Probe error={rateLimited(2)} />)
    await expect.element(screen.getByTestId('remaining')).toHaveTextContent('2')
  })

  it('stays at zero for non-429 errors', async () => {
    const screen = await render(
      <Probe
        error={new ApiError({ code: 'invalid_credentials', status: 401 })}
      />
    )
    await expect.element(screen.getByTestId('remaining')).toHaveTextContent('0')
  })

  it('stays at zero when there is no error', async () => {
    vi.useFakeTimers()
    const screen = await render(<Probe error={null} />)

    advance(5000)
    await expect.element(screen.getByTestId('remaining')).toHaveTextContent('0')
  })
})

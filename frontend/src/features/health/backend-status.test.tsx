import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import i18n from '@/i18n'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { apiClient } from '@/api/mutator'
import { BackendStatus } from './backend-status'

function renderStatus() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })

  return render(
    <QueryClientProvider client={queryClient}>
      <BackendStatus />
    </QueryClientProvider>
  )
}

describe('BackendStatus', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('calls /health/ready through the generated client and reports ready', async () => {
    const request = vi
      .spyOn(apiClient, 'request')
      .mockResolvedValue({ data: { status: 'ready' } } as never)

    const screen = await renderStatus()

    await expect
      .element(screen.getByRole('status'))
      .toHaveTextContent(i18n.t('backendStatus.ready'))

    expect(request).toHaveBeenCalledTimes(1)
    expect(request.mock.calls[0]?.[0]).toMatchObject({
      url: '/health/ready',
      method: 'GET',
    })
  })

  it('reports an unreachable backend when the readiness request fails', async () => {
    vi.spyOn(apiClient, 'request').mockRejectedValue(
      new Error('connection refused')
    )

    const screen = await renderStatus()

    await expect
      .element(screen.getByRole('status'))
      .toHaveTextContent(i18n.t('backendStatus.unreachable'))
  })
})

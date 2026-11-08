import i18n from '@/i18n'
import { describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import { RequestId } from './request-id'

describe('RequestId', () => {
  it('shows the request id and copies it to the clipboard', async () => {
    const writeText = vi
      .spyOn(navigator.clipboard, 'writeText')
      .mockResolvedValue()

    const { getByText, getByRole } = await render(
      <RequestId requestId='01JABCDEF' />
    )

    await expect.element(getByText('01JABCDEF')).toBeInTheDocument()

    await userEvent.click(
      getByRole('button', { name: i18n.t('requestId.copy') })
    )

    expect(writeText).toHaveBeenCalledWith('01JABCDEF')
    await expect
      .element(getByText(i18n.t('requestId.copied')))
      .toBeInTheDocument()
  })

  it('renders nothing without a request id', async () => {
    const { container } = await render(<RequestId requestId={undefined} />)
    expect(container.innerHTML).toBe('')
  })
})

import i18n from '@/i18n'
import { createReauthTestController, reauthRequired } from '@/test-utils/reauth'
import { describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import { ApiError } from '@/lib/api-error'
import { ReauthCancelledError } from './reauth-controller'
import { ReauthDialog } from './reauth-dialog'

function setup() {
  const { controller, deps } = createReauthTestController()
  const settled = vi.fn()

  function Harness({ operation }: { operation: () => Promise<unknown> }) {
    return (
      <>
        <button
          type='button'
          onClick={() => {
            void controller.runWithReauth(operation).then(
              (value) => settled(null, value),
              (error) => settled(error)
            )
          }}
        >
          save
        </button>
        <ReauthDialog controller={controller} />
      </>
    )
  }

  return { controller, deps, settled, Harness }
}

async function submitPassword(
  screen: Awaited<ReturnType<typeof render>>,
  password: string
) {
  await userEvent.fill(
    screen.getByLabelText(i18n.t('auth:reauth.password'), { exact: true }),
    password
  )
  await userEvent.click(
    screen.getByRole('button', { name: i18n.t('auth:reauth.submit') })
  )
}

describe('ReauthDialog', () => {
  it('prompts for the current password and retries the operation', async () => {
    const { controller, deps, settled, Harness } = setup()
    const operation = vi
      .fn()
      .mockRejectedValueOnce(reauthRequired())
      .mockResolvedValueOnce('saved')
    const screen = await render(<Harness operation={operation} />)

    await userEvent.click(screen.getByRole('button', { name: 'save' }))

    await expect.element(screen.getByRole('dialog')).toBeInTheDocument()

    await submitPassword(screen, 'current-password')

    await vi.waitFor(() => expect(settled).toHaveBeenCalledWith(null, 'saved'))
    expect(deps.reauthenticate).toHaveBeenCalledWith('current-password')
    expect(deps.applyAccessToken).toHaveBeenCalledWith('fresh-token')
    expect(operation).toHaveBeenCalledTimes(2)
    await expect.element(screen.getByRole('dialog')).not.toBeInTheDocument()
    expect(controller.getSnapshot().open).toBe(false)
  })

  it('keeps invalid_current_password inline and lets the user retry', async () => {
    const { deps, settled, Harness } = setup()
    deps.reauthenticate.mockRejectedValueOnce(
      new ApiError({ code: 'invalid_current_password', status: 422 })
    )
    const operation = vi
      .fn()
      .mockRejectedValueOnce(reauthRequired())
      .mockResolvedValueOnce('saved')
    const screen = await render(<Harness operation={operation} />)

    await userEvent.click(screen.getByRole('button', { name: 'save' }))
    await submitPassword(screen, 'wrong-password')

    await expect
      .element(screen.getByText(i18n.t('errors:auth.invalid_current_password')))
      .toBeInTheDocument()
    await expect.element(screen.getByRole('dialog')).toBeInTheDocument()
    expect(operation).toHaveBeenCalledTimes(1)

    await submitPassword(screen, 'right-password')

    await vi.waitFor(() => expect(settled).toHaveBeenCalledWith(null, 'saved'))
    expect(deps.reauthenticate).toHaveBeenLastCalledWith('right-password')
    expect(operation).toHaveBeenCalledTimes(2)
  })

  it('shows the Retry-After countdown and disables submit', async () => {
    const { deps, Harness } = setup()
    deps.reauthenticate.mockRejectedValueOnce(
      new ApiError(
        { code: 'rate_limited', status: 429 },
        { retryAfterSeconds: 1 }
      )
    )
    const operation = vi.fn().mockRejectedValueOnce(reauthRequired())
    const screen = await render(<Harness operation={operation} />)

    await userEvent.click(screen.getByRole('button', { name: 'save' }))
    await submitPassword(screen, 'current-password')

    const alert = screen.getByRole('alert')
    await expect
      .element(alert)
      .toHaveTextContent(i18n.t('errors:auth.rate_limited'))
    await expect
      .element(alert)
      .toHaveTextContent(i18n.t('common:error.retryIn', { seconds: 1 }))

    const submit = screen.getByRole('button', {
      name: i18n.t('auth:reauth.submit'),
    })
    await expect.element(submit).toBeDisabled()

    await new Promise((resolve) => setTimeout(resolve, 1100))
    await expect.element(submit).toBeEnabled()
  })

  it('cancels without running the operation again', async () => {
    const { settled, Harness } = setup()
    const operation = vi.fn().mockRejectedValueOnce(reauthRequired())
    const screen = await render(<Harness operation={operation} />)

    await userEvent.click(screen.getByRole('button', { name: 'save' }))
    await expect.element(screen.getByRole('dialog')).toBeInTheDocument()

    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('common:actions.cancel') })
    )

    await vi.waitFor(() =>
      expect(settled).toHaveBeenCalledWith(expect.any(ReauthCancelledError))
    )
    expect(operation).toHaveBeenCalledTimes(1)
    await expect.element(screen.getByRole('dialog')).not.toBeInTheDocument()
  })

  it('closes when reauthentication is refused for a restricted session', async () => {
    const { deps, settled, Harness } = setup()
    const passwordChangeRequired = new ApiError({
      code: 'password_change_required',
      status: 403,
    })
    deps.reauthenticate.mockRejectedValueOnce(passwordChangeRequired)
    const operation = vi.fn().mockRejectedValueOnce(reauthRequired())
    const screen = await render(<Harness operation={operation} />)

    await userEvent.click(screen.getByRole('button', { name: 'save' }))
    await submitPassword(screen, 'current-password')

    await vi.waitFor(() =>
      expect(settled).toHaveBeenCalledWith(passwordChangeRequired)
    )
    await expect.element(screen.getByRole('dialog')).not.toBeInTheDocument()
    expect(operation).toHaveBeenCalledTimes(1)
  })
})

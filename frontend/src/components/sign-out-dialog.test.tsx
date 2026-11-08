import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import { SignOutDialog } from './sign-out-dialog'

const navigate = vi.fn()
const signOutOfApp = vi.fn(async () => ({ ok: true }))

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>()
  return { ...actual, useNavigate: () => navigate }
})

vi.mock('@/features/auth/sign-out', () => ({
  signOutOfApp: () => signOutOfApp(),
}))

describe('SignOutDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    signOutOfApp.mockResolvedValue({ ok: true })
  })

  it('signs out and returns to the sign-in page', async () => {
    const { getByRole } = await render(
      <SignOutDialog open onOpenChange={vi.fn()} />
    )

    await userEvent.click(getByRole('button', { name: /^Sign out$/i }))

    await vi.waitFor(() =>
      expect(navigate).toHaveBeenCalledWith({
        to: '/sign-in',
        replace: true,
      })
    )
    expect(signOutOfApp).toHaveBeenCalledTimes(1)
  })

  it('still leaves the page when local sign-out throws', async () => {
    signOutOfApp.mockRejectedValueOnce(new Error('unexpected'))
    const { getByRole } = await render(
      <SignOutDialog open onOpenChange={vi.fn()} />
    )

    await userEvent.click(getByRole('button', { name: /^Sign out$/i }))

    await vi.waitFor(() =>
      expect(navigate).toHaveBeenCalledWith({
        to: '/sign-in',
        replace: true,
      })
    )
  })

  it('does not sign out or navigate when Cancel is clicked', async () => {
    const { getByRole } = await render(
      <SignOutDialog open onOpenChange={vi.fn()} />
    )

    await userEvent.click(getByRole('button', { name: /^Cancel$/i }))

    expect(signOutOfApp).not.toHaveBeenCalled()
    expect(navigate).not.toHaveBeenCalled()
  })
})

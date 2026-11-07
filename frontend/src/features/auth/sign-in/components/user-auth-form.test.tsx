import { describe, expect, it } from 'vitest'
import { render } from 'vitest-browser-react'
import { UserAuthForm } from './user-auth-form'

describe('UserAuthForm', () => {
  it('renders identifier and password fields with a disabled submit button', async () => {
    const screen = await render(<UserAuthForm />)

    await expect
      .element(screen.getByRole('textbox', { name: /^Username or email$/i }))
      .toBeInTheDocument()
    await expect
      .element(screen.getByLabelText(/^Password$/i))
      .toBeInTheDocument()

    const submit = screen.getByRole('button', { name: /^Sign in$/i })
    await expect.element(submit).toBeDisabled()

    await expect
      .element(
        screen.getByText(
          /Sign-in is not connected to the backend yet, so submissions are disabled\./i
        )
      )
      .toBeInTheDocument()
  })

  it('does not render removed sign-up or forgot-password links', async () => {
    const screen = await render(<UserAuthForm />)

    await expect.element(screen.getByText(/Sign up/i)).not.toBeInTheDocument()
    await expect
      .element(screen.getByText(/Forgot password/i))
      .not.toBeInTheDocument()
  })
})

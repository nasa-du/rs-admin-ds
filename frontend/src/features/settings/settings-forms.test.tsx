import { describe, expect, it } from 'vitest'
import { render } from 'vitest-browser-react'
import { FontProvider } from '@/context/font-provider'
import { ThemeProvider } from '@/context/theme-provider'
import { AccountForm } from './account/account-form'
import { AppearanceForm } from './appearance/appearance-form'
import { DisplayForm } from './display/display-form'
import { NotificationsForm } from './notifications/notifications-form'
import { ProfileForm } from './profile/profile-form'

describe('settings forms', () => {
  it('renders the profile form', async () => {
    const screen = await render(<ProfileForm />)

    await expect.element(screen.getByLabelText('Username')).toBeInTheDocument()
    await expect.element(screen.getByLabelText('Email')).toBeInTheDocument()
    await expect.element(screen.getByLabelText('Bio')).toBeInTheDocument()
  })

  it('renders the account form with only the supported languages', async () => {
    const screen = await render(<AccountForm />)

    await expect.element(screen.getByLabelText('Name')).toBeInTheDocument()
    await expect
      .element(screen.getByText('Date of birth', { exact: true }))
      .toBeInTheDocument()
    await expect
      .element(screen.getByRole('combobox', { name: /Language/ }))
      .toBeInTheDocument()
  })

  it('renders the notifications form', async () => {
    const screen = await render(<NotificationsForm />)

    await expect
      .element(screen.getByText('Email Notifications'))
      .toBeInTheDocument()
    await expect
      .element(screen.getByLabelText('Communication emails'))
      .toBeInTheDocument()
  })

  it('renders the display form', async () => {
    const screen = await render(<DisplayForm />)

    await expect
      .element(
        screen.getByText('Select the items you want to display in the sidebar.')
      )
      .toBeInTheDocument()
    await expect.element(screen.getByLabelText('Recents')).toBeInTheDocument()
  })

  it('renders the appearance form', async () => {
    const screen = await render(
      <ThemeProvider>
        <FontProvider>
          <AppearanceForm />
        </FontProvider>
      </ThemeProvider>
    )

    await expect.element(screen.getByLabelText('Font')).toBeInTheDocument()
    await expect
      .element(screen.getByText('Theme', { exact: true }))
      .toBeInTheDocument()
    await expect.element(screen.getByText('Light')).toBeInTheDocument()
    await expect.element(screen.getByText('Dark')).toBeInTheDocument()
  })
})

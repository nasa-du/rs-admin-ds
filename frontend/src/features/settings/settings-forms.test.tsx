import i18n from '@/i18n'
import { authWrapper, makeUser } from '@/test-utils/auth'
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
  it('renders the profile form with administrator-managed identity fields', async () => {
    const screen = await render(<ProfileForm />, {
      wrapper: authWrapper(makeUser({ username: 'alice' })),
    })

    await expect
      .element(screen.getByLabelText(i18n.t('settings:profile.displayName')))
      .toBeInTheDocument()
    await expect
      .element(screen.getByText(i18n.t('settings:profile.username')))
      .toBeInTheDocument()
    await expect
      .element(screen.getByText(i18n.t('settings:profile.email')))
      .toBeInTheDocument()
    await expect
      .element(screen.getByText(i18n.t('settings:profile.roles')))
      .toBeInTheDocument()
  })

  it('renders the language picker with only the supported languages', async () => {
    const screen = await render(<AccountForm />)

    await expect
      .element(
        screen.getByRole('combobox', {
          name: i18n.t('settings:account.language'),
        })
      )
      .toBeInTheDocument()
    await expect
      .element(screen.getByText(i18n.t('settings:account.languageHint')))
      .toBeInTheDocument()
  })

  it('labels notifications as browser-only preferences', async () => {
    const screen = await render(<NotificationsForm />)

    await expect
      .element(screen.getByText(i18n.t('settings:notifications.localOnly')))
      .toBeInTheDocument()
    await expect
      .element(screen.getByText(i18n.t('settings:notifications.emailTitle')))
      .toBeInTheDocument()
    await expect
      .element(
        screen.getByLabelText(i18n.t('settings:notifications.communication'))
      )
      .toBeInTheDocument()
  })

  it('labels display preferences as browser-only', async () => {
    const screen = await render(<DisplayForm />)

    await expect
      .element(screen.getByText(i18n.t('settings:display.localOnly')))
      .toBeInTheDocument()
    await expect
      .element(screen.getByLabelText(i18n.t('settings:display.items.recents')))
      .toBeInTheDocument()
  })

  it('labels appearance preferences as browser-only', async () => {
    const screen = await render(
      <ThemeProvider>
        <FontProvider>
          <AppearanceForm />
        </FontProvider>
      </ThemeProvider>
    )

    await expect
      .element(screen.getByText(i18n.t('settings:appearance.localOnly')))
      .toBeInTheDocument()
    await expect
      .element(screen.getByLabelText(i18n.t('settings:appearance.font')))
      .toBeInTheDocument()
    await expect
      .element(
        screen.getByText(i18n.t('settings:appearance.theme'), { exact: true })
      )
      .toBeInTheDocument()
  })
})

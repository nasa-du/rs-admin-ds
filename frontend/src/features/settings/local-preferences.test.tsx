import i18n from '@/i18n'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import { DisplayForm } from './display/display-form'
import { NotificationsForm } from './notifications/notifications-form'

const NOTIFICATIONS_KEY = 'rs-admin:notifications'
const DISPLAY_KEY = 'rs-admin:display'

afterEach(() => {
  localStorage.removeItem(NOTIFICATIONS_KEY)
  localStorage.removeItem(DISPLAY_KEY)
})

describe('local preference forms', () => {
  it('states that notifications are browser-only and never sent in v1', async () => {
    const screen = await render(<NotificationsForm />)

    await expect
      .element(screen.getByText(i18n.t('settings:notifications.localOnly')))
      .toBeInTheDocument()
    expect(localStorage.getItem(NOTIFICATIONS_KEY)).toBeNull()
  })

  it('persists notification preferences locally', async () => {
    const screen = await render(<NotificationsForm />)

    const marketing = screen.getByLabelText(
      i18n.t('settings:notifications.marketing')
    )
    await userEvent.click(marketing)
    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('settings:notifications.submit'),
      })
    )

    await vi.waitFor(() => {
      const stored = JSON.parse(localStorage.getItem(NOTIFICATIONS_KEY) ?? '{}')
      expect(stored.marketing_emails).toBe(true)
    })
  })

  it('restores notification preferences from local storage', async () => {
    localStorage.setItem(
      NOTIFICATIONS_KEY,
      JSON.stringify({
        type: 'mentions',
        communication_emails: true,
        marketing_emails: false,
        social_emails: false,
        security_emails: true,
        mobile: true,
      })
    )

    const screen = await render(<NotificationsForm />)

    await expect
      .element(
        screen.getByLabelText(i18n.t('settings:notifications.marketing'))
      )
      .not.toBeChecked()
    await expect
      .element(
        screen.getByLabelText(i18n.t('settings:notifications.communication'))
      )
      .toBeChecked()
  })

  it('states that display preferences are browser-only and persists them', async () => {
    const screen = await render(<DisplayForm />)

    await expect
      .element(screen.getByText(i18n.t('settings:display.localOnly')))
      .toBeInTheDocument()

    await userEvent.click(
      screen.getByLabelText(i18n.t('settings:display.items.home'))
    )
    await userEvent.click(
      screen.getByRole('button', { name: i18n.t('settings:display.submit') })
    )

    await vi.waitFor(() => {
      const stored = JSON.parse(localStorage.getItem(DISPLAY_KEY) ?? '{}')
      expect(stored.items).toEqual(['recents'])
    })
  })
})

import i18n from '@/i18n'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import { getCookie } from '@/lib/cookies'
import { LocaleProvider } from '@/components/locale-provider'
import { AccountForm } from './account-form'
import { SignOutAllDevices } from './sign-out-all-devices'

const navigate = vi.fn()
const signOutEverywhere = vi.fn(async () => ({ ok: true }))

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>()
  return { ...actual, useNavigate: () => navigate }
})

vi.mock('@/features/auth/sign-out', () => ({
  signOutEverywhere: () => signOutEverywhere(),
}))

beforeEach(() => {
  vi.clearAllMocks()
  signOutEverywhere.mockResolvedValue({ ok: true })
})

describe('AccountForm', () => {
  it('switches language immediately and persists it to the locale cookie', async () => {
    const screen = await render(
      <LocaleProvider>
        <AccountForm />
      </LocaleProvider>
    )

    await userEvent.click(
      screen.getByRole('combobox', {
        name: i18n.t('settings:account.language'),
      })
    )
    await userEvent.click(
      screen.getByRole('option', {
        name: i18n.t('common:language.zh-CN'),
      })
    )

    await vi.waitFor(() => expect(i18n.resolvedLanguage).toBe('zh-CN'))
    expect(getCookie('locale')).toBe('zh-CN')
    await vi.waitFor(() =>
      expect(document.documentElement.getAttribute('lang')).toBe('zh-CN')
    )
    await expect
      .element(screen.getByText(i18n.t('common:language.zh-CN')))
      .toBeInTheDocument()
  })
})

describe('SignOutAllDevices', () => {
  it('confirms, revokes every session, and returns to sign-in', async () => {
    const screen = await render(<SignOutAllDevices />)

    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('settings:account.signOutAll'),
      })
    )
    await expect.element(screen.getByRole('alertdialog')).toBeInTheDocument()

    await userEvent.click(
      screen
        .getByRole('alertdialog')
        .getByRole('button', { name: i18n.t('settings:account.signOutAll') })
    )

    await vi.waitFor(() => expect(signOutEverywhere).toHaveBeenCalledTimes(1))
    await vi.waitFor(() =>
      expect(navigate).toHaveBeenCalledWith({
        to: '/sign-in',
        replace: true,
      })
    )
  })

  it('still leaves for sign-in when the server request fails locally', async () => {
    signOutEverywhere.mockResolvedValueOnce({ ok: false })
    const screen = await render(<SignOutAllDevices />)

    await userEvent.click(
      screen.getByRole('button', {
        name: i18n.t('settings:account.signOutAll'),
      })
    )
    await userEvent.click(
      screen
        .getByRole('alertdialog')
        .getByRole('button', { name: i18n.t('settings:account.signOutAll') })
    )

    await vi.waitFor(() =>
      expect(navigate).toHaveBeenCalledWith({
        to: '/sign-in',
        replace: true,
      })
    )
  })
})

import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { SettingsAccount } from '@/features/settings/account'

export const Route = createFileRoute('/_authenticated/settings/account')({
  component: SettingsAccount,
  staticData: { titleKey: PAGE_TITLE_KEYS.settingsAccount },
})

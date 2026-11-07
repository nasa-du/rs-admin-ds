import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { SettingsNotifications } from '@/features/settings/notifications'

export const Route = createFileRoute('/_authenticated/settings/notifications')({
  component: SettingsNotifications,
  staticData: { titleKey: PAGE_TITLE_KEYS.settingsNotifications },
})

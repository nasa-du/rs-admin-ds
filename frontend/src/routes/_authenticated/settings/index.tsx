import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { SettingsProfile } from '@/features/settings/profile'

export const Route = createFileRoute('/_authenticated/settings/')({
  component: SettingsProfile,
  staticData: { titleKey: PAGE_TITLE_KEYS.settingsProfile },
})

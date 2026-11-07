import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { SettingsAppearance } from '@/features/settings/appearance'

export const Route = createFileRoute('/_authenticated/settings/appearance')({
  component: SettingsAppearance,
  staticData: { titleKey: PAGE_TITLE_KEYS.settingsAppearance },
})

import { createFileRoute } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { SettingsDisplay } from '@/features/settings/display'

export const Route = createFileRoute('/_authenticated/settings/display')({
  component: SettingsDisplay,
  staticData: { titleKey: PAGE_TITLE_KEYS.settingsDisplay },
})

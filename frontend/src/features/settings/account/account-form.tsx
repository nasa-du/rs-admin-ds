import {
  SUPPORTED_LOCALES,
  isSupportedLocale,
  setLocale,
  type Locale,
} from '@/i18n'
import { LANGUAGE_LABEL_KEYS } from '@/i18n/languages'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'

/**
 * Language is a local preference (ADR-0006 §8): it is written to the `locale`
 * cookie and never sent to the server.
 */
export function AccountForm() {
  const { t, i18n } = useTranslation('settings')
  const { t: tCommon } = useTranslation('common')

  const resolved = i18n.resolvedLanguage ?? i18n.language
  const currentLocale: Locale =
    resolved && isSupportedLocale(resolved) ? resolved : 'zh-CN'

  async function handleLocaleChange(value: string) {
    if (!isSupportedLocale(value) || value === currentLocale) return
    await setLocale(value)
    toast.success(t('account.languageSaved'))
  }

  return (
    <section className='space-y-4'>
      <div className='space-y-0.5'>
        <h3 className='text-lg font-medium'>{t('account.languageTitle')}</h3>
        <p className='text-sm text-muted-foreground'>
          {t('account.languageDescription')}
        </p>
      </div>

      <div className='grid gap-2'>
        <Label htmlFor='settings-language'>{t('account.language')}</Label>
        <Select value={currentLocale} onValueChange={handleLocaleChange}>
          <SelectTrigger
            id='settings-language'
            aria-label={t('account.language')}
            className='w-50'
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {SUPPORTED_LOCALES.map((locale) => (
              <SelectItem key={locale} value={locale}>
                {tCommon(LANGUAGE_LABEL_KEYS[locale])}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <p className='text-sm text-muted-foreground'>
          {t('account.languageHint')}
        </p>
      </div>
    </section>
  )
}

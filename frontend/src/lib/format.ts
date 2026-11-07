import i18n from '@/i18n'

type DateFormatPreset = 'date' | 'dateTime' | 'shortDate' | 'time'

export type FormatDateOptions = {
  locale?: string
  preset?: DateFormatPreset
}

const DATE_PRESETS: Record<DateFormatPreset, Intl.DateTimeFormatOptions> = {
  date: { dateStyle: 'medium' },
  dateTime: { dateStyle: 'medium', timeStyle: 'short' },
  shortDate: { year: 'numeric', month: '2-digit', day: '2-digit' },
  time: { timeStyle: 'short' },
}

function currentLocale(locale?: string): string {
  return locale ?? i18n.resolvedLanguage ?? i18n.language ?? 'zh-CN'
}

/**
 * Format a date with `Intl.DateTimeFormat`, using the active locale unless one
 * is supplied explicitly.
 */
export function formatDate(
  value: Date | string | number,
  { locale, preset = 'date' }: FormatDateOptions = {}
): string {
  const date = value instanceof Date ? value : new Date(value)
  if (Number.isNaN(date.getTime())) return ''

  return new Intl.DateTimeFormat(currentLocale(locale), {
    ...DATE_PRESETS[preset],
  }).format(date)
}

export function formatNumber(
  value: number,
  options: Intl.NumberFormatOptions & { locale?: string } = {}
): string {
  const { locale, ...numberOptions } = options
  return new Intl.NumberFormat(currentLocale(locale), numberOptions).format(
    value
  )
}

export function formatCurrency(
  value: number,
  currency: string,
  options: Intl.NumberFormatOptions & { locale?: string } = {}
): string {
  const { locale, ...numberOptions } = options
  return new Intl.NumberFormat(currentLocale(locale), {
    style: 'currency',
    currency,
    ...numberOptions,
  }).format(value)
}

export function formatPercent(
  value: number,
  options: Intl.NumberFormatOptions & { locale?: string } = {}
): string {
  const { locale, ...numberOptions } = options
  return new Intl.NumberFormat(currentLocale(locale), {
    style: 'percent',
    ...numberOptions,
  }).format(value)
}

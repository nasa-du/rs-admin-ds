import i18n from '@/i18n'

type DateFormatPreset =
  'date' | 'dateTime' | 'dateTimeSeconds' | 'shortDate' | 'time'

export type FormatDateOptions = {
  locale?: string
  preset?: DateFormatPreset
}

const DATE_PRESETS: Record<DateFormatPreset, Intl.DateTimeFormatOptions> = {
  date: { dateStyle: 'medium' },
  dateTime: { dateStyle: 'medium', timeStyle: 'short' },
  // Audit events need second precision (spec §9.10).
  dateTimeSeconds: {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  },
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

/**
 * Display instant with seconds; unparsable values fall back to the raw value
 * so an audit row never renders an empty cell.
 */
export function formatAuditTimestamp(value: string, locale?: string): string {
  return formatDate(value, { locale, preset: 'dateTimeSeconds' }) || value
}

/** RFC3339 UTC value used for tooltips and copying. */
export function rfc3339Utc(value: string): string {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return date.toISOString()
}

/** `UTC+08:00`-style label for the browser time zone. */
export function formatTimezoneLabel(date: Date = new Date()): string {
  const offsetMinutes = -date.getTimezoneOffset()
  const sign = offsetMinutes >= 0 ? '+' : '-'
  const absolute = Math.abs(offsetMinutes)
  const hours = String(Math.floor(absolute / 60)).padStart(2, '0')
  const minutes = String(absolute % 60).padStart(2, '0')
  return `UTC${sign}${hours}:${minutes}`
}

function pad(value: number): string {
  return String(value).padStart(2, '0')
}

/** ISO instant → `YYYY-MM-DDTHH:mm` in the browser time zone. */
export function toDateTimeLocalValue(iso: string): string {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return ''

  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(
    date.getDate()
  )}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}

/** `datetime-local` value (browser time zone) → ISO UTC instant. */
export function parseDateTimeLocalValue(value: string): string | null {
  if (!value) return null
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return null
  return date.toISOString()
}

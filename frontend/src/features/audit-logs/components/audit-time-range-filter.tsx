import { useState } from 'react'
import { RefreshCw } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import {
  formatTimezoneLabel,
  parseDateTimeLocalValue,
  toDateTimeLocalValue,
} from '@/lib/format'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { presetWindowPatch, type AuditLogsSearch } from '../audit-logs-search'
import {
  AUDIT_RANGE_PRESETS,
  isAuditRangePreset,
  isValidAuditRange,
  type AuditRangePresetId,
  type AuditWindow,
} from '../audit-time'

const PRESET_KEYS = {
  '15m': 'filters.presets.15m',
  '1h': 'filters.presets.1h',
  '24h': 'filters.presets.24h',
  '7d': 'filters.presets.7d',
  '30d': 'filters.presets.30d',
} as const satisfies Record<AuditRangePresetId, string>

type CustomRangeError = 'invalidDateTime' | 'invalidRange' | 'rangeTooLarge'

const ERROR_KEYS = {
  invalidDateTime: 'filters.invalidDateTime',
  invalidRange: 'filters.invalidRange',
  rangeTooLarge: 'filters.rangeTooLarge',
} as const satisfies Record<CustomRangeError, string>

type AuditTimeRangeFilterProps = {
  search: AuditLogsSearch
  auditWindow: AuditWindow
  onChange: (patch: Partial<AuditLogsSearch>) => void
}

/**
 * Absolute time-window control (spec §9.10). Presets are resolved to an
 * absolute UTC `from`/`to` pair immediately; a custom range is validated
 * before it is written to the URL. The server still rejects bad ranges.
 */
export function AuditTimeRangeFilter({
  search,
  auditWindow,
  onChange,
}: AuditTimeRangeFilterProps) {
  const { t } = useTranslation('audit')
  const [customOpen, setCustomOpen] = useState(false)
  const [fromInput, setFromInput] = useState('')
  const [toInput, setToInput] = useState('')
  const [error, setError] = useState<CustomRangeError | null>(null)

  function openCustomRange() {
    setFromInput(toDateTimeLocalValue(auditWindow.from))
    setToInput(toDateTimeLocalValue(auditWindow.to))
    setError(null)
    setCustomOpen(true)
  }

  function applyCustomRange() {
    const from = parseDateTimeLocalValue(fromInput)
    const to = parseDateTimeLocalValue(toInput)

    if (!from || !to) {
      setError('invalidDateTime')
      return
    }
    if (Date.parse(from) >= Date.parse(to)) {
      setError('invalidRange')
      return
    }
    if (!isValidAuditRange(from, to)) {
      setError('rangeTooLarge')
      return
    }

    onChange({ from, to, range: undefined, page: undefined, detail: undefined })
    setCustomOpen(false)
  }

  const activeRange = search.range
  const recalculateLabel = activeRange
    ? t('filters.recalculate', { preset: t(PRESET_KEYS[activeRange]) })
    : ''

  return (
    <div className='flex items-center gap-1'>
      <Select
        value={activeRange ?? 'custom'}
        onValueChange={(value) => {
          if (value === 'custom') {
            openCustomRange()
            return
          }
          if (isAuditRangePreset(value)) {
            onChange(presetWindowPatch(value))
          }
        }}
      >
        <SelectTrigger
          size='sm'
          className='h-8 w-44'
          aria-label={t('filters.timeRange')}
        >
          <SelectValue placeholder={t('filters.timeRange')} />
        </SelectTrigger>
        <SelectContent>
          {AUDIT_RANGE_PRESETS.map((preset) => (
            <SelectItem key={preset.id} value={preset.id}>
              {t(PRESET_KEYS[preset.id])}
            </SelectItem>
          ))}
          <SelectItem value='custom'>{t('filters.custom')}</SelectItem>
        </SelectContent>
      </Select>

      {activeRange && (
        <Button
          type='button'
          variant='ghost'
          size='sm'
          className='h-8 px-2'
          aria-label={recalculateLabel}
          title={recalculateLabel}
          onClick={() => onChange(presetWindowPatch(activeRange))}
        >
          <RefreshCw className='size-4' />
        </Button>
      )}

      <Dialog open={customOpen} onOpenChange={setCustomOpen}>
        <DialogContent className='sm:max-w-md'>
          <DialogHeader>
            <DialogTitle>{t('filters.custom')}</DialogTitle>
            <DialogDescription>
              {t('filters.timezone', {
                timezone: formatTimezoneLabel(),
              })}
            </DialogDescription>
          </DialogHeader>

          <div className='grid gap-3'>
            <div className='grid gap-1.5'>
              <Label htmlFor='audit-range-from'>{t('filters.from')}</Label>
              <Input
                id='audit-range-from'
                type='datetime-local'
                value={fromInput}
                step={60}
                onChange={(event) => setFromInput(event.target.value)}
              />
            </div>
            <div className='grid gap-1.5'>
              <Label htmlFor='audit-range-to'>{t('filters.to')}</Label>
              <Input
                id='audit-range-to'
                type='datetime-local'
                value={toInput}
                step={60}
                onChange={(event) => setToInput(event.target.value)}
              />
            </div>
            {error && (
              <p role='alert' className='text-sm text-destructive'>
                {t(ERROR_KEYS[error])}
              </p>
            )}
          </div>

          <DialogFooter>
            <Button
              type='button'
              variant='outline'
              onClick={() => setCustomOpen(false)}
            >
              {t('filters.cancel')}
            </Button>
            <Button type='button' onClick={applyCustomRange}>
              {t('filters.apply')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}

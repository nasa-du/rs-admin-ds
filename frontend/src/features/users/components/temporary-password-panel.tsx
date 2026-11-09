import { useState } from 'react'
import { Check, Copy } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { formatDate } from '@/lib/format'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { copyToClipboard } from '../copy-to-clipboard'

type TemporaryPasswordPanelProps = {
  password: string
  expiresAt: string
}

/**
 * One-time temporary-password display (spec §9.8). The value lives only in
 * the parent dialog's React state; it is never written to a query cache,
 * storage, or an analytics event, and it disappears when the dialog closes.
 */
export function TemporaryPasswordPanel({
  password,
  expiresAt,
}: TemporaryPasswordPanelProps) {
  const { t } = useTranslation('users')
  const [copied, setCopied] = useState(false)

  async function copyPassword() {
    if (await copyToClipboard(password)) {
      setCopied(true)
      toast.success(t('password.copied'))
      return
    }
    toast.error(t('password.copyFailed'))
  }

  return (
    <div className='grid gap-4'>
      <div
        role='alert'
        className='rounded-md border border-amber-500/50 bg-amber-500/10 p-3 text-sm'
      >
        {t('password.warning')}
      </div>

      <div className='grid gap-2'>
        <Label htmlFor='temporary-password-view'>{t('password.title')}</Label>
        <div className='flex gap-2'>
          <Input
            id='temporary-password-view'
            readOnly
            value={password}
            autoComplete='off'
            spellCheck={false}
            className='font-mono'
            onFocus={(event) => event.currentTarget.select()}
          />
          <Button
            type='button'
            variant='outline'
            size='icon'
            onClick={() => void copyPassword()}
            aria-label={t('password.copy')}
          >
            {copied ? <Check /> : <Copy />}
          </Button>
        </div>
        <p className='text-sm text-muted-foreground'>
          {t('password.expiresAt', {
            date: formatDate(expiresAt, { preset: 'dateTime' }),
          })}
        </p>
      </div>
    </div>
  )
}

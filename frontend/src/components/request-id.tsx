import { useState } from 'react'
import { Check, Copy } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'

type RequestIdProps = {
  requestId: string | null | undefined
}

/**
 * Copyable `request_id` for troubleshooting (spec §9.12). Renders nothing when
 * the response did not include one.
 */
export function RequestId({ requestId }: RequestIdProps) {
  const { t } = useTranslation('common')
  const [copied, setCopied] = useState(false)

  if (!requestId) return null

  async function copy() {
    if (!requestId) return
    try {
      await navigator.clipboard.writeText(requestId)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      // Clipboard access can be denied; the id is still selectable as text.
    }
  }

  return (
    <div className='flex flex-wrap items-center gap-2 text-xs text-muted-foreground'>
      <span>
        {t('requestId.label')}: <code>{requestId}</code>
      </span>
      <Button
        type='button'
        variant='ghost'
        size='sm'
        className='h-6 px-2'
        aria-label={t('requestId.copy')}
        onClick={() => void copy()}
      >
        {copied ? <Check /> : <Copy />}
        {copied ? t('requestId.copied') : t('requestId.copy')}
      </Button>
    </div>
  )
}

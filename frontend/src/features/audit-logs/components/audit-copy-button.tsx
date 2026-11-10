import type { ReactNode } from 'react'
import { Copy } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { copyToClipboard } from '@/lib/clipboard'
import { Button } from '@/components/ui/button'

type AuditCopyButtonProps = {
  value: string
  label: string
  variant?: 'ghost' | 'outline'
  size?: 'icon' | 'sm'
  className?: string
  /** Stop the click from opening the surrounding table row. */
  stopPropagation?: boolean
  children?: ReactNode
}

/** Copy-to-clipboard action shared by the audit list and detail Sheet. */
export function AuditCopyButton({
  value,
  label,
  variant = 'ghost',
  size = 'icon',
  className,
  stopPropagation = false,
  children,
}: AuditCopyButtonProps) {
  const { t } = useTranslation('audit')

  async function copy(event: React.MouseEvent<HTMLButtonElement>) {
    if (stopPropagation) event.stopPropagation()
    if (await copyToClipboard(value)) {
      toast.success(t('detail.copied'))
      return
    }
    toast.error(t('detail.copyFailed'))
  }

  return (
    <Button
      type='button'
      variant={variant}
      size={size}
      className={className}
      aria-label={label}
      onClick={(event) => void copy(event)}
    >
      {children ?? <Copy className='size-3.5' />}
    </Button>
  )
}

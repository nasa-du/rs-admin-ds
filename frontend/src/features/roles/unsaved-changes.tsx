import { ConfirmDialog } from '@/components/confirm-dialog'
import type { UnsavedBlocker } from './use-unsaved-changes'

type UnsavedChangesDialogProps = {
  blocker: UnsavedBlocker
  title: string
  description: string
  discardLabel: string
  continueLabel: string
}

export function UnsavedChangesDialog({
  blocker,
  title,
  description,
  discardLabel,
  continueLabel,
}: UnsavedChangesDialogProps) {
  if (blocker.status !== 'blocked') return null

  return (
    <ConfirmDialog
      open
      onOpenChange={(open) => {
        if (!open) blocker.reset?.()
      }}
      title={title}
      desc={description}
      confirmText={discardLabel}
      cancelBtnText={continueLabel}
      destructive
      handleConfirm={() => blocker.proceed?.()}
    />
  )
}

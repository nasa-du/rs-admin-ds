/**
 * Marks a settings form as a browser-only preference (spec §9.11). The notice
 * is shared by Appearance, Notifications and Display.
 */
export function LocalOnlyNotice({ children }: { children: React.ReactNode }) {
  return (
    <p className='rounded-md border bg-muted/50 p-3 text-sm text-muted-foreground'>
      {children}
    </p>
  )
}

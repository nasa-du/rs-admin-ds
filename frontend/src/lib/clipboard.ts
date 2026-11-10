/** Clipboard writes fail silently in unsupported or permission-denied contexts. */
export async function copyToClipboard(value: string): Promise<boolean> {
  if (!navigator.clipboard) return false
  try {
    await navigator.clipboard.writeText(value)
    return true
  } catch {
    return false
  }
}

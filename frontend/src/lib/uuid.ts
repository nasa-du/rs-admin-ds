export const UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

/** Whether a value looks like a UUID (format only; the server validates). */
export function isUuid(value: string): boolean {
  return UUID_PATTERN.test(value)
}

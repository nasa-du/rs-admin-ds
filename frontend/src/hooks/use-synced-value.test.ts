import { describe, expect, it } from 'vitest'
import { renderHook } from 'vitest-browser-react'
import { useSyncedValue } from './use-synced-value'

describe('useSyncedValue', () => {
  it('starts from the URL value and keeps local edits while the URL is unchanged', async () => {
    const { result, rerender, act } = await renderHook(
      ({ value }: { value: string } = { value: '' }) => useSyncedValue(value),
      { initialProps: { value: 'alice' } }
    )

    expect(result.current[0]).toBe('alice')

    await act(() => result.current[1]('alice b'))
    expect(result.current[0]).toBe('alice b')

    await rerender({ value: 'alice' })
    expect(result.current[0]).toBe('alice b')
  })

  it('resets the local value when the URL value changes externally', async () => {
    const { result, rerender } = await renderHook(
      ({ value }: { value: string } = { value: '' }) => useSyncedValue(value),
      { initialProps: { value: '' } }
    )

    await rerender({ value: 'bob' })

    expect(result.current[0]).toBe('bob')
  })
})

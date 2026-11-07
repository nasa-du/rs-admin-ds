import i18n from '@/i18n'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { LocaleProvider } from './locale-provider'

describe('LocaleProvider', () => {
  afterEach(async () => {
    await i18n.changeLanguage('en-US')
  })

  it('syncs <html lang> with the active locale and keeps LTR', async () => {
    await i18n.changeLanguage('zh-CN')

    await render(
      <LocaleProvider>
        <div>content</div>
      </LocaleProvider>
    )

    await vi.waitFor(() =>
      expect(document.documentElement.getAttribute('lang')).toBe('zh-CN')
    )
    expect(document.documentElement.getAttribute('dir')).toBe('ltr')

    await i18n.changeLanguage('en-US')

    await vi.waitFor(() =>
      expect(document.documentElement.getAttribute('lang')).toBe('en-US')
    )
    expect(document.documentElement.getAttribute('dir')).toBe('ltr')
  })
})

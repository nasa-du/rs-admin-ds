import i18n from '@/i18n'
import { afterEach, beforeEach } from 'vitest'
import { clearCookies } from './cookies'

/**
 * Component tests keep the English resource active so their assertions stay
 * readable. i18n-specific tests switch to zh-CN explicitly and restore en-US.
 */
beforeEach(async () => {
  clearCookies()
  await i18n.changeLanguage('en-US')
})

afterEach(async () => {
  clearCookies()
  await i18n.changeLanguage('en-US')
})

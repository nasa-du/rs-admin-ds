import type audit from './locales/zh-CN/audit.json'
import type auth from './locales/zh-CN/auth.json'
import type common from './locales/zh-CN/common.json'
import type errors from './locales/zh-CN/errors.json'
import type nav from './locales/zh-CN/nav.json'
import type roles from './locales/zh-CN/roles.json'
import type settings from './locales/zh-CN/settings.json'
import type users from './locales/zh-CN/users.json'
import type validation from './locales/zh-CN/validation.json'

declare module 'i18next' {
  interface CustomTypeOptions {
    defaultNS: 'common'
    resources: {
      common: typeof common
      nav: typeof nav
      auth: typeof auth
      users: typeof users
      roles: typeof roles
      audit: typeof audit
      settings: typeof settings
      errors: typeof errors
      validation: typeof validation
    }
  }
}

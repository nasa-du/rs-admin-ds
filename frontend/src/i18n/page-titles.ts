/**
 * Static page-title keys used by route `staticData` and `DocumentTitle`.
 *
 * Keeping these as literal constants avoids building translation keys at
 * runtime from route data.
 */
export const PAGE_TITLE_KEYS = {
  dashboard: 'nav:dashboard',
  users: 'users:title',
  settings: 'settings:title',
  settingsProfile: 'settings:nav.profile',
  settingsAccount: 'settings:nav.account',
  settingsAppearance: 'settings:nav.appearance',
  settingsNotifications: 'settings:nav.notifications',
  settingsDisplay: 'settings:nav.display',
  signIn: 'auth:signIn.title',
  unauthorized: 'errors:pages.unauthorized.title',
  forbidden: 'errors:pages.forbidden.title',
  notFound: 'errors:pages.notFound.title',
  generalError: 'errors:pages.general.title',
  maintenance: 'errors:pages.maintenance.title',
} as const

export type PageTitleKey =
  (typeof PAGE_TITLE_KEYS)[keyof typeof PAGE_TITLE_KEYS]

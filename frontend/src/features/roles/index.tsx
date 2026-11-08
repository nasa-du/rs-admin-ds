import { useTranslation } from 'react-i18next'
import { PlaceholderPage } from '@/components/placeholder-page'

export function Roles() {
  const { t } = useTranslation('roles')

  return (
    <PlaceholderPage
      title={t('title')}
      description={t('description')}
      emptyTitle={t('empty.title')}
      emptyDescription={t('empty.description')}
    />
  )
}

export function RoleCreate() {
  const { t } = useTranslation('roles')

  return (
    <PlaceholderPage
      title={t('new.title')}
      description={t('new.description')}
      emptyTitle={t('empty.title')}
      emptyDescription={t('empty.description')}
    />
  )
}

export function RoleDetail() {
  const { t } = useTranslation('roles')

  return (
    <PlaceholderPage
      title={t('detail.title')}
      description={t('detail.description')}
      emptyTitle={t('empty.title')}
      emptyDescription={t('empty.description')}
    />
  )
}

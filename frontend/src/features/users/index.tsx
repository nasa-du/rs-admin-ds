import { useTranslation } from 'react-i18next'
import { PlaceholderPage } from '@/components/placeholder-page'

export function Users() {
  const { t } = useTranslation('users')

  return (
    <PlaceholderPage
      title={t('title')}
      description={t('description')}
      emptyTitle={t('empty.title')}
      emptyDescription={t('empty.description')}
    />
  )
}

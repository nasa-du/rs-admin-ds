import { useTranslation } from 'react-i18next'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { ConfigDrawer } from '@/components/config-drawer'
import { Header } from '@/components/layout/header'
import { Main } from '@/components/layout/main'
import { ProfileDropdown } from '@/components/profile-dropdown'
import { Search } from '@/components/search'
import { ThemeSwitch } from '@/components/theme-switch'

export function Users() {
  const { t } = useTranslation('users')

  return (
    <>
      <Header fixed>
        <Search className='me-auto' />
        <ThemeSwitch />
        <ConfigDrawer />
        <ProfileDropdown />
      </Header>

      <Main className='flex flex-1 flex-col gap-4 sm:gap-6'>
        <div className='flex flex-wrap items-end justify-between gap-2'>
          <div>
            <h2 className='text-2xl font-bold tracking-tight'>{t('title')}</h2>
            <p className='text-muted-foreground'>{t('description')}</p>
          </div>
        </div>
        <Card className='max-w-2xl'>
          <CardHeader>
            <CardTitle>{t('empty.title')}</CardTitle>
            <CardDescription>{t('empty.description')}</CardDescription>
          </CardHeader>
          <CardContent />
        </Card>
      </Main>
    </>
  )
}

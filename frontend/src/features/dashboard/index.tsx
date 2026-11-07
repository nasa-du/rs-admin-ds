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

export function Dashboard() {
  const { t } = useTranslation('common')
  const { t: tNav } = useTranslation('nav')

  return (
    <>
      {/* ===== Top Heading ===== */}
      <Header>
        <Search className='me-auto' />
        <ThemeSwitch />
        <ConfigDrawer />
        <ProfileDropdown />
      </Header>

      {/* ===== Main ===== */}
      <Main>
        <div className='mb-2 flex items-center justify-between space-y-2'>
          <h1 className='text-2xl font-bold tracking-tight'>
            {tNav('dashboard')}
          </h1>
        </div>
        <Card className='max-w-2xl'>
          <CardHeader>
            <CardTitle>{t('welcome.title')}</CardTitle>
            <CardDescription>{t('welcome.description')}</CardDescription>
          </CardHeader>
          <CardContent />
        </Card>
      </Main>
    </>
  )
}

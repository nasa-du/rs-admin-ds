import { Link } from '@tanstack/react-router'
import { ScrollText, Settings, ShieldCheck, Users } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
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
import { Can } from '@/features/auth/can'

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
          <CardContent className='space-y-3'>
            <p className='text-sm font-medium'>{t('welcome.quickLinks')}</p>
            <div className='flex flex-wrap gap-2'>
              <Can permission='users.read'>
                <Button variant='outline' size='sm' asChild>
                  <Link to='/users'>
                    <Users className='size-4' />
                    {tNav('users')}
                  </Link>
                </Button>
              </Can>
              <Can permission='roles.read'>
                <Button variant='outline' size='sm' asChild>
                  <Link to='/roles'>
                    <ShieldCheck className='size-4' />
                    {tNav('roles')}
                  </Link>
                </Button>
              </Can>
              <Can permission='audit.read'>
                <Button variant='outline' size='sm' asChild>
                  <Link to='/audit-logs'>
                    <ScrollText className='size-4' />
                    {tNav('auditLogs')}
                  </Link>
                </Button>
              </Can>
              <Button variant='outline' size='sm' asChild>
                <Link to='/settings'>
                  <Settings className='size-4' />
                  {tNav('settings')}
                </Link>
              </Button>
            </div>
          </CardContent>
        </Card>
      </Main>
    </>
  )
}

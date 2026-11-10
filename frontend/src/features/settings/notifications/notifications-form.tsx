import { useMemo } from 'react'
import { z } from 'zod'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { type TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import {
  readLocalPreference,
  writeLocalPreference,
} from '@/lib/local-preference'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { Switch } from '@/components/ui/switch'
import { LocalOnlyNotice } from '../components/local-only-notice'

function makeNotificationsFormSchema(t: TFunction<'validation'>) {
  return z.object({
    type: z.enum(['all', 'mentions', 'none'], {
      error: () => t('selectNotificationType'),
    }),
    mobile: z.boolean().default(false).optional(),
    communication_emails: z.boolean().default(false).optional(),
    social_emails: z.boolean().default(false).optional(),
    marketing_emails: z.boolean().default(false).optional(),
    security_emails: z.boolean(),
  })
}

type NotificationsFormValues = z.infer<
  ReturnType<typeof makeNotificationsFormSchema>
>

const STORAGE_KEY = 'rs-admin:notifications'

const DEFAULT_VALUES: NotificationsFormValues = {
  type: 'all',
  communication_emails: false,
  marketing_emails: false,
  social_emails: true,
  security_emails: true,
}

export function NotificationsForm() {
  const { t } = useTranslation('settings')
  const { t: tValidation } = useTranslation('validation')
  const schema = useMemo(
    () => makeNotificationsFormSchema(tValidation),
    [tValidation]
  )

  const form = useForm<NotificationsFormValues>({
    resolver: zodResolver(schema),
    defaultValues: readLocalPreference(STORAGE_KEY, DEFAULT_VALUES),
  })

  return (
    <Form {...form}>
      <form
        onSubmit={form.handleSubmit((values) => {
          writeLocalPreference(STORAGE_KEY, values)
          toast.success(t('notifications.saved'))
        })}
        className='space-y-8'
      >
        <LocalOnlyNotice>{t('notifications.localOnly')}</LocalOnlyNotice>
        <FormField
          control={form.control}
          name='type'
          render={({ field }) => (
            <FormItem className='relative space-y-3'>
              <FormLabel>{t('notifications.type')}</FormLabel>
              <FormControl>
                <RadioGroup
                  onValueChange={field.onChange}
                  defaultValue={field.value}
                  className='flex flex-col gap-2'
                >
                  <FormItem className='flex items-center'>
                    <FormControl>
                      <RadioGroupItem value='all' />
                    </FormControl>
                    <FormLabel className='font-normal'>
                      {t('notifications.all')}
                    </FormLabel>
                  </FormItem>
                  <FormItem className='flex items-center'>
                    <FormControl>
                      <RadioGroupItem value='mentions' />
                    </FormControl>
                    <FormLabel className='font-normal'>
                      {t('notifications.mentions')}
                    </FormLabel>
                  </FormItem>
                  <FormItem className='flex items-center'>
                    <FormControl>
                      <RadioGroupItem value='none' />
                    </FormControl>
                    <FormLabel className='font-normal'>
                      {t('notifications.none')}
                    </FormLabel>
                  </FormItem>
                </RadioGroup>
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <div className='relative'>
          <h3 className='mb-4 text-lg font-medium'>
            {t('notifications.emailTitle')}
          </h3>
          <div className='space-y-4'>
            <FormField
              control={form.control}
              name='communication_emails'
              render={({ field }) => (
                <FormItem className='flex flex-row items-center justify-between rounded-lg border p-4'>
                  <div className='space-y-0.5'>
                    <FormLabel className='text-base'>
                      {t('notifications.communication')}
                    </FormLabel>
                    <FormDescription>
                      {t('notifications.communicationHint')}
                    </FormDescription>
                  </div>
                  <FormControl>
                    <Switch
                      checked={field.value}
                      onCheckedChange={field.onChange}
                    />
                  </FormControl>
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name='marketing_emails'
              render={({ field }) => (
                <FormItem className='flex flex-row items-center justify-between rounded-lg border p-4'>
                  <div className='space-y-0.5'>
                    <FormLabel className='text-base'>
                      {t('notifications.marketing')}
                    </FormLabel>
                    <FormDescription>
                      {t('notifications.marketingHint')}
                    </FormDescription>
                  </div>
                  <FormControl>
                    <Switch
                      checked={field.value}
                      onCheckedChange={field.onChange}
                    />
                  </FormControl>
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name='social_emails'
              render={({ field }) => (
                <FormItem className='flex flex-row items-center justify-between rounded-lg border p-4'>
                  <div className='space-y-0.5'>
                    <FormLabel className='text-base'>
                      {t('notifications.social')}
                    </FormLabel>
                    <FormDescription>
                      {t('notifications.socialHint')}
                    </FormDescription>
                  </div>
                  <FormControl>
                    <Switch
                      checked={field.value}
                      onCheckedChange={field.onChange}
                    />
                  </FormControl>
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name='security_emails'
              render={({ field }) => (
                <FormItem className='flex flex-row items-center justify-between rounded-lg border p-4'>
                  <div className='space-y-0.5'>
                    <FormLabel className='text-base'>
                      {t('notifications.security')}
                    </FormLabel>
                    <FormDescription>
                      {t('notifications.securityHint')}
                    </FormDescription>
                  </div>
                  <FormControl>
                    <Switch
                      checked={field.value}
                      onCheckedChange={field.onChange}
                      disabled
                      aria-readonly
                    />
                  </FormControl>
                </FormItem>
              )}
            />
          </div>
        </div>
        <FormField
          control={form.control}
          name='mobile'
          render={({ field }) => (
            <FormItem className='relative flex flex-row items-start'>
              <FormControl>
                <Checkbox
                  checked={field.value}
                  onCheckedChange={field.onChange}
                />
              </FormControl>
              <div className='space-y-1 leading-none'>
                <FormLabel>{t('notifications.mobile')}</FormLabel>
                <FormDescription>
                  {t('notifications.mobileHint')}
                </FormDescription>
              </div>
            </FormItem>
          )}
        />
        <Button type='submit'>{t('notifications.submit')}</Button>
      </form>
    </Form>
  )
}

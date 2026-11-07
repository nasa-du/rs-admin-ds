import { useMemo } from 'react'
import { z } from 'zod'
import { useFieldArray, useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { type TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { cn } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'

function makeProfileFormSchema(t: TFunction<'validation'>) {
  return z.object({
    username: z
      .string()
      .min(2, t('usernameMin', { min: 2 }))
      .max(30, t('usernameMax', { max: 30 })),
    email: z.email({ error: () => t('invalid_email') }),
    bio: z
      .string()
      .min(4, t('bioMin', { min: 4 }))
      .max(160, t('max_length', { max: 160 })),
    urls: z
      .array(
        z.object({
          value: z.url(t('invalid_url')),
        })
      )
      .optional(),
  })
}

type ProfileFormValues = z.infer<ReturnType<typeof makeProfileFormSchema>>

export function ProfileForm() {
  const { t } = useTranslation('settings')
  const { t: tValidation } = useTranslation('validation')
  const { t: tCommon } = useTranslation('common')
  const schema = useMemo(
    () => makeProfileFormSchema(tValidation),
    [tValidation]
  )

  const form = useForm<ProfileFormValues>({
    resolver: zodResolver(schema),
    defaultValues: {
      username: '',
      email: '',
      bio: '',
      urls: [],
    },
    mode: 'onChange',
  })

  const { fields, append } = useFieldArray({
    name: 'urls',
    control: form.control,
  })

  return (
    <Form {...form}>
      <form
        onSubmit={form.handleSubmit(() => {
          toast.info(tCommon('notAvailableShort'))
        })}
        className='space-y-8'
      >
        <FormField
          control={form.control}
          name='username'
          render={({ field }) => (
            <FormItem>
              <FormLabel>{t('profile.username')}</FormLabel>
              <FormControl>
                <Input
                  placeholder={t('profile.usernamePlaceholder')}
                  {...field}
                />
              </FormControl>
              <FormDescription>{t('profile.usernameHint')}</FormDescription>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name='email'
          render={({ field }) => (
            <FormItem>
              <FormLabel>{t('profile.email')}</FormLabel>
              <FormControl>
                <Input
                  type='email'
                  placeholder={t('profile.emailPlaceholder')}
                  {...field}
                />
              </FormControl>
              <FormDescription>{t('profile.emailHint')}</FormDescription>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name='bio'
          render={({ field }) => (
            <FormItem>
              <FormLabel>{t('profile.bio')}</FormLabel>
              <FormControl>
                <Textarea
                  placeholder={t('profile.bioPlaceholder')}
                  className='resize-none'
                  {...field}
                />
              </FormControl>
              <FormDescription>{t('profile.bioHint')}</FormDescription>
              <FormMessage />
            </FormItem>
          )}
        />
        <div>
          {fields.map((field, index) => (
            <FormField
              control={form.control}
              key={field.id}
              name={`urls.${index}.value`}
              render={({ field }) => (
                <FormItem>
                  <FormLabel className={cn(index !== 0 && 'sr-only')}>
                    {t('profile.urls')}
                  </FormLabel>
                  <FormDescription className={cn(index !== 0 && 'sr-only')}>
                    {t('profile.urlsHint')}
                  </FormDescription>
                  <FormControl className={cn(index !== 0 && 'mt-1.5')}>
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
          ))}
          <Button
            type='button'
            variant='outline'
            size='sm'
            className='mt-2'
            onClick={() => append({ value: '' })}
          >
            {t('profile.addUrl')}
          </Button>
        </div>
        <Button type='submit'>{t('profile.submit')}</Button>
      </form>
    </Form>
  )
}

import { useMemo } from 'react'
import { z } from 'zod'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { type TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
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

const items = [
  {
    id: 'recents',
    titleKey: 'display.items.recents',
  },
  {
    id: 'home',
    titleKey: 'display.items.home',
  },
  {
    id: 'applications',
    titleKey: 'display.items.applications',
  },
  {
    id: 'desktop',
    titleKey: 'display.items.desktop',
  },
  {
    id: 'downloads',
    titleKey: 'display.items.downloads',
  },
  {
    id: 'documents',
    titleKey: 'display.items.documents',
  },
] as const

function makeDisplayFormSchema(t: TFunction<'validation'>) {
  return z.object({
    items: z.array(z.string()).refine((value) => value.some((item) => item), {
      message: t('selectAtLeastOneItem'),
    }),
  })
}

type DisplayFormValues = z.infer<ReturnType<typeof makeDisplayFormSchema>>

export function DisplayForm() {
  const { t } = useTranslation('settings')
  const { t: tValidation } = useTranslation('validation')
  const schema = useMemo(
    () => makeDisplayFormSchema(tValidation),
    [tValidation]
  )

  const form = useForm<DisplayFormValues>({
    resolver: zodResolver(schema),
    defaultValues: {
      items: ['recents', 'home'],
    },
  })

  return (
    <Form {...form}>
      <form
        onSubmit={form.handleSubmit(() => {
          toast.success(t('display.saved'))
        })}
        className='space-y-8'
      >
        <FormField
          control={form.control}
          name='items'
          render={() => (
            <FormItem>
              <div className='mb-4'>
                <FormLabel className='text-base'>
                  {t('display.sidebar')}
                </FormLabel>
                <FormDescription>{t('display.sidebarHint')}</FormDescription>
              </div>
              {items.map((item) => (
                <FormField
                  key={item.id}
                  control={form.control}
                  name='items'
                  render={({ field }) => {
                    return (
                      <FormItem
                        key={item.id}
                        className='flex flex-row items-start'
                      >
                        <FormControl>
                          <Checkbox
                            checked={field.value?.includes(item.id)}
                            onCheckedChange={(checked) => {
                              return checked
                                ? field.onChange([...field.value, item.id])
                                : field.onChange(
                                    field.value?.filter(
                                      (value) => value !== item.id
                                    )
                                  )
                            }}
                          />
                        </FormControl>
                        <FormLabel className='font-normal'>
                          {t(item.titleKey)}
                        </FormLabel>
                      </FormItem>
                    )
                  }}
                />
              ))}
              <FormMessage />
            </FormItem>
          )}
        />
        <Button type='submit'>{t('display.submit')}</Button>
      </form>
    </Form>
  )
}

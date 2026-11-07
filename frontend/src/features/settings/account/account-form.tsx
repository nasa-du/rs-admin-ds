import { useMemo } from 'react'
import { z } from 'zod'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import {
  SUPPORTED_LOCALES,
  isSupportedLocale,
  setLocale,
  type Locale,
} from '@/i18n'
import { LANGUAGE_LABEL_KEYS } from '@/i18n/languages'
import { type TFunction } from 'i18next'
import { Check, ChevronsUpDown } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { cn } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
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
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { DatePicker } from '@/components/date-picker'

function makeAccountFormSchema(t: TFunction<'validation'>) {
  return z.object({
    name: z
      .string()
      .min(2, t('nameMin', { min: 2 }))
      .max(30, t('nameMax', { max: 30 })),
    dob: z.date({ error: () => t('required') }),
    language: z.enum(SUPPORTED_LOCALES, {
      error: () => t('selectLanguage'),
    }),
  })
}

type AccountFormValues = z.infer<ReturnType<typeof makeAccountFormSchema>>

export function AccountForm() {
  const { t, i18n } = useTranslation('settings')
  const { t: tValidation } = useTranslation('validation')
  const { t: tCommon } = useTranslation('common')
  const schema = useMemo(
    () => makeAccountFormSchema(tValidation),
    [tValidation]
  )

  const resolvedLanguage = i18n.resolvedLanguage
  const currentLocale: Locale =
    resolvedLanguage && isSupportedLocale(resolvedLanguage)
      ? resolvedLanguage
      : 'zh-CN'

  const languages = SUPPORTED_LOCALES.map((locale) => ({
    value: locale,
    label: tCommon(LANGUAGE_LABEL_KEYS[locale]),
  }))

  const form = useForm<AccountFormValues>({
    resolver: zodResolver(schema),
    defaultValues: {
      name: '',
      language: currentLocale,
    },
  })

  function onSubmit() {
    toast.success(t('account.saved'))
  }

  return (
    <Form {...form}>
      <form onSubmit={form.handleSubmit(onSubmit)} className='space-y-8'>
        <FormField
          control={form.control}
          name='name'
          render={({ field }) => (
            <FormItem>
              <FormLabel>{t('account.name')}</FormLabel>
              <FormControl>
                <Input placeholder={t('account.namePlaceholder')} {...field} />
              </FormControl>
              <FormDescription>{t('account.nameHint')}</FormDescription>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name='dob'
          render={({ field }) => (
            <FormItem className='flex flex-col'>
              <FormLabel>{t('account.dob')}</FormLabel>
              <DatePicker selected={field.value} onSelect={field.onChange} />
              <FormDescription>{t('account.dobHint')}</FormDescription>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name='language'
          render={({ field }) => (
            <FormItem className='flex flex-col'>
              <FormLabel>{t('account.language')}</FormLabel>
              <Popover>
                <PopoverTrigger asChild>
                  <FormControl>
                    <Button
                      variant='outline'
                      role='combobox'
                      className={cn(
                        'w-50 justify-between',
                        !field.value && 'text-muted-foreground'
                      )}
                    >
                      {field.value
                        ? tCommon(LANGUAGE_LABEL_KEYS[field.value])
                        : t('account.languagePlaceholder')}
                      <ChevronsUpDown className='ms-2 h-4 w-4 shrink-0 opacity-50' />
                    </Button>
                  </FormControl>
                </PopoverTrigger>
                <PopoverContent className='w-50 p-0'>
                  <Command>
                    <CommandInput placeholder={t('account.languageSearch')} />
                    <CommandEmpty>{t('account.languageEmpty')}</CommandEmpty>
                    <CommandGroup>
                      <CommandList>
                        {languages.map((language) => (
                          <CommandItem
                            value={language.label}
                            key={language.value}
                            onSelect={() => {
                              form.setValue('language', language.value)
                              void setLocale(language.value)
                            }}
                          >
                            <Check
                              className={cn(
                                'size-4',
                                language.value === field.value
                                  ? 'opacity-100'
                                  : 'opacity-0'
                              )}
                            />
                            {language.label}
                          </CommandItem>
                        ))}
                      </CommandList>
                    </CommandGroup>
                  </Command>
                </PopoverContent>
              </Popover>
              <FormDescription>{t('account.languageHint')}</FormDescription>
              <FormMessage />
            </FormItem>
          )}
        />
        <Button type='submit'>{t('account.submit')}</Button>
      </form>
    </Form>
  )
}

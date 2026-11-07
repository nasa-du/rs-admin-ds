import { useMemo } from 'react'
import { z } from 'zod'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { type TFunction } from 'i18next'
import { LogIn } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { Input } from '@/components/ui/input'
import { PasswordInput } from '@/components/password-input'

function makeSignInSchema(t: TFunction<'validation'>) {
  return z.object({
    identifier: z.string().min(1, t('required')),
    password: z.string().min(1, t('required')),
  })
}

type SignInFormValues = z.infer<ReturnType<typeof makeSignInSchema>>

export function UserAuthForm({
  className,
  ...props
}: React.HTMLAttributes<HTMLFormElement>) {
  const { t } = useTranslation('auth')
  const { t: tValidation } = useTranslation('validation')
  const schema = useMemo(() => makeSignInSchema(tValidation), [tValidation])

  const form = useForm<SignInFormValues>({
    resolver: zodResolver(schema),
    defaultValues: {
      identifier: '',
      password: '',
    },
  })

  return (
    <Form {...form}>
      <form
        onSubmit={form.handleSubmit(() => undefined)}
        className={cn('grid gap-3', className)}
        {...props}
      >
        <FormField
          control={form.control}
          name='identifier'
          render={({ field }) => (
            <FormItem>
              <FormLabel>{t('signIn.identifier')}</FormLabel>
              <FormControl>
                <Input
                  autoComplete='username'
                  placeholder={t('signIn.identifierPlaceholder')}
                  {...field}
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name='password'
          render={({ field }) => (
            <FormItem className='relative'>
              <FormLabel>{t('signIn.password')}</FormLabel>
              <FormControl>
                <PasswordInput
                  autoComplete='current-password'
                  placeholder={t('signIn.passwordPlaceholder')}
                  {...field}
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <Button className='mt-2' disabled>
          <LogIn />
          {t('signIn.submit')}
        </Button>
        <p className='text-center text-sm text-muted-foreground'>
          {t('signIn.unavailable')}
        </p>
      </form>
    </Form>
  )
}

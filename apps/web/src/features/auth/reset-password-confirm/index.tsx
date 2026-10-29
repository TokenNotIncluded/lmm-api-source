/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import { zodResolver } from '@hookform/resolvers/zod'
import { Loader2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useForm } from 'react-hook-form'
import { useTranslation } from 'react-i18next'
import type { z } from 'zod'

import { PasswordInput } from '@/components/password-input'
import { Alert, AlertDescription } from '@/components/ui/alert'
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
import { Label } from '@/components/ui/label'
import { resetPassword } from '@/features/auth/api'
import {
  PASSWORD_RESET_COUNTDOWN,
  resetPasswordFormSchema,
} from '@/features/auth/constants'
import { useCountdown } from '@/hooks/use-countdown'

import { AuthLayout } from '../auth-layout'

declare global {
  interface Window {
    __lmmEmailReset?: { email: string; token: string }
  }
}

type ResetState =
  | { kind: 'invalid' }
  | { kind: 'ready'; email: string; token: string }
  | { kind: 'complete' }

export function ResetPasswordConfirm() {
  const { t } = useTranslation()
  const [reset, setReset] = useState<ResetState>(() =>
    window.__lmmEmailReset
      ? { kind: 'ready', ...window.__lmmEmailReset }
      : { kind: 'invalid' }
  )
  // Clear after initialization so React StrictMode can initialize twice.
  useEffect(() => {
    delete window.__lmmEmailReset
  }, [])
  const [loading, setLoading] = useState(false)
  const [failed, setFailed] = useState(false)
  const form = useForm<z.infer<typeof resetPasswordFormSchema>>({
    resolver: zodResolver(resetPasswordFormSchema),
    defaultValues: { password: '', confirmPassword: '' },
  })
  const {
    secondsLeft,
    isActive,
    start: startCountdown,
  } = useCountdown({ initialSeconds: PASSWORD_RESET_COUNTDOWN })

  async function handleSubmit(values: z.infer<typeof resetPasswordFormSchema>) {
    if (reset.kind !== 'ready' || loading || isActive) return

    startCountdown()
    setLoading(true)
    setFailed(false)
    try {
      const res = await resetPassword({
        email: reset.email,
        token: reset.token,
        new_password: values.password,
      })
      if (res.success) {
        form.reset()
        setReset({ kind: 'complete' })
      } else {
        setFailed(true)
      }
    } catch {
      setFailed(true)
    } finally {
      setLoading(false)
    }
  }

  return (
    <AuthLayout reloadDocument>
      <div className='w-full space-y-8'>
        <div className='space-y-2'>
          <h2 className='text-center text-2xl font-semibold tracking-tight sm:text-left'>
            {t('Reset password')}
          </h2>
          <p className='text-muted-foreground text-left text-sm sm:text-base'>
            {reset.kind === 'complete'
              ? t('auth.resetPasswordConfirm.success')
              : t('auth.resetPasswordConfirm.description')}
          </p>
        </div>

        <div className='space-y-4'>
          {reset.kind === 'invalid' && (
            <Alert variant='destructive'>
              <AlertDescription>
                {t('Invalid reset link, please request a new password reset.')}
              </AlertDescription>
            </Alert>
          )}

          {failed && (
            <Alert variant='destructive'>
              <AlertDescription>{t('Reset failed')}</AlertDescription>
            </Alert>
          )}

          {reset.kind === 'ready' && (
            <Form {...form}>
              <form
                onSubmit={form.handleSubmit(handleSubmit)}
                className='space-y-4'
              >
                <div className='space-y-2'>
                  <Label htmlFor='email'>{t('Email')}</Label>
                  <Input id='email' type='email' value={reset.email} disabled />
                </div>
                <FormField
                  control={form.control}
                  name='password'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('New password')}</FormLabel>
                      <FormControl>
                        <PasswordInput
                          autoComplete='new-password'
                          disabled={loading}
                          {...field}
                        />
                      </FormControl>
                      <FormDescription>
                        {t('Enter password (8-20 characters)')}
                      </FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name='confirmPassword'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('Confirm password')}</FormLabel>
                      <FormControl>
                        <PasswordInput
                          autoComplete='new-password'
                          disabled={loading}
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <Button
                  type='submit'
                  className='h-auto min-h-11 w-full whitespace-normal'
                  disabled={loading || isActive}
                >
                  {loading && <Loader2 className='animate-spin' />}
                  {isActive
                    ? t('auth.resetPasswordConfirm.retry', {
                        seconds: secondsLeft,
                      })
                    : t('auth.resetPasswordConfirm.confirm')}
                </Button>
              </form>
            </Form>
          )}

          <Button
            type='button'
            variant={reset.kind === 'complete' ? 'default' : 'link'}
            className='w-full'
            onClick={() => window.location.replace('/sign-in')}
          >
            {reset.kind === 'complete'
              ? t('auth.resetPasswordConfirm.backToLogin')
              : t('Back to login')}
          </Button>
        </div>
      </div>
    </AuthLayout>
  )
}

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
import { Link } from '@tanstack/react-router'
import axios from 'axios'
import { Loader2, KeyRound } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useForm } from 'react-hook-form'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import type { z } from 'zod'

import { Dialog } from '@/components/dialog'
import { PasswordInput } from '@/components/password-input'
import { Turnstile } from '@/components/turnstile'
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
import { Label } from '@/components/ui/label'
import { login } from '@/features/auth/api'
import { HumanGrantButton } from '@/features/auth/components/humangrant-button'
import { LegalConsent } from '@/features/auth/components/legal-consent'
import { OAuthProviders } from '@/features/auth/components/oauth-providers'
import { loginFormSchema } from '@/features/auth/constants'
import { useAuthRedirect } from '@/features/auth/hooks/use-auth-redirect'
import { useFederatedWechat } from '@/features/auth/hooks/use-federated-wechat'
import { useTurnstile } from '@/features/auth/hooks/use-turnstile'
import {
  completeAuthResponse,
  verifyAuthPasskey,
  reportAuthError,
} from '@/features/auth/lib/auth-completion'
import { federatedProviders } from '@/features/auth/lib/federated-oauth'
import { hasOAuthLoginProvider } from '@/features/auth/lib/registration'
import { beginPasskeyLogin } from '@/features/auth/passkey'
import type { AuthFormProps } from '@/features/auth/types'
import { AccountClosureRecovery } from '@/features/profile/components/account-closure-recovery'
import { useStatus } from '@/hooks/use-status'
import { capturePasswordLoginGuard } from '@/lib/auth-session'
import { isPasskeySupported as detectPasskeySupport } from '@/lib/passkey'
import { cn } from '@/lib/utils'

import { AccountAppealForm } from './account-appeal-form'

export function UserAuthForm({
  className,
  redirectTo,
  ...props
}: AuthFormProps) {
  const { t } = useTranslation()
  const [isLoading, setIsLoading] = useState(false)
  const [appealRefreshKey, setAppealRefreshKey] = useState(0)
  const [passkeySupported, setPasskeySupported] = useState(false)
  const [isPasskeyLoading, setIsPasskeyLoading] = useState(false)
  const [turnstileWidgetKey, setTurnstileWidgetKey] = useState(0)
  const legalConsentErrorMessage = t('Please agree to the legal terms first')
  const loginFailedMessage = t('Login failed')

  const { status, capabilitiesReady } = useStatus()
  const passkeyLoginEnabled = Boolean(
    status?.passkey_login ?? status?.data?.passkey_login
  )
  const passwordLoginEnabled =
    (status?.password_login_enabled ??
      status?.data?.password_login_enabled ??
      true) !== false
  const {
    isTurnstileEnabled,
    turnstileSiteKey,
    turnstileToken,
    setTurnstileToken,
    resetTurnstile,
    validateTurnstile,
  } = useTurnstile()
  const { handleLoginSuccess, redirectTo2FA } = useAuthRedirect()
  const pendingLogin = useRef<AbortController | null>(null)
  useEffect(() => () => pendingLogin.current?.abort(), [])

  const hasUserAgreement = Boolean(status?.user_agreement_enabled)
  const hasPrivacyPolicy = Boolean(status?.privacy_policy_enabled)
  const requiresLegalConsent = hasUserAgreement || hasPrivacyPolicy
  const [agreedToLegal, setAgreedToLegal] = useState(!requiresLegalConsent)
  const passkeyButtonDisabled =
    isLoading ||
    isPasskeyLoading ||
    !passkeySupported ||
    (requiresLegalConsent && !agreedToLegal)
  const hasWeChatLogin = federatedProviders(status).some(
    (provider) => provider.kind === 'wechat'
  )
  const hasOAuthLogin = hasOAuthLoginProvider(status)
  const hasAlternativeLogin =
    passkeyLoginEnabled || hasWeChatLogin || hasOAuthLogin

  const [previousLegalConsent, setPreviousLegalConsent] =
    useState(requiresLegalConsent)
  if (previousLegalConsent !== requiresLegalConsent) {
    setPreviousLegalConsent(requiresLegalConsent)
    setAgreedToLegal(!requiresLegalConsent)
  }

  useEffect(() => {
    detectPasskeySupport()
      .then(setPasskeySupported)
      .catch(() => setPasskeySupported(false))
  }, [])

  const form = useForm<z.infer<typeof loginFormSchema>>({
    resolver: zodResolver(loginFormSchema),
    defaultValues: {
      username: '',
      password: '',
    },
  })

  const {
    wechatCode,
    setWeChatCode,
    isWeChatDialogOpen,
    isWeChatSubmitting,
    wechatQrCodeUrl,
    handleOpenWeChatDialog,
    handleWeChatDialogChange,
    handleWeChatLogin,
  } = useFederatedWechat(
    status,
    agreedToLegal,
    requiresLegalConsent,
    redirectTo,
    refreshVerifiedAccountAppeal
  )

  async function onSubmit(data: z.infer<typeof loginFormSchema>) {
    if (!passwordLoginEnabled) return
    if (requiresLegalConsent && !agreedToLegal) {
      toast.error(legalConsentErrorMessage)
      return
    }

    if (!validateTurnstile()) return

    if (pendingLogin.current) return
    const controller = new AbortController()
    pendingLogin.current = controller
    setIsLoading(true)
    try {
      const res = await login(
        {
          username: data.username,
          password: data.password,
          turnstile: turnstileToken,
        },
        controller.signal
      )
      if (controller.signal.aborted) return

      const handled = await completeAuthResponse(res, {
        alreadyApplied: true,
        signal: controller.signal,
        onMFA: () => redirectTo2FA(redirectTo),
        onBundle: async (bundle) => {
          await handleLoginSuccess(bundle, redirectTo, true, controller.signal)
          if (!controller.signal.aborted) toast.success(t('Welcome back!'))
        },
      })
      if (!handled) {
        reportAuthError(
          res,
          t,
          loginFailedMessage,
          () => {
            setAppealRefreshKey((current) => current + 1)
            toast.error(t('This user account is disabled.'))
            return true
          },
          'password'
        )
      }
    } catch (error) {
      if (controller.signal.aborted || axios.isCancel(error)) return
      reportAuthError(
        error,
        t,
        loginFailedMessage,
        refreshVerifiedAccountAppeal,
        'password'
      )
    } finally {
      if (pendingLogin.current === controller) pendingLogin.current = null
      setTurnstileToken('')
      setTurnstileWidgetKey((current) => current + 1)
      setIsLoading(false)
    }
  }

  function refreshVerifiedAccountAppeal() {
    setAppealRefreshKey((current) => current + 1)
  }

  async function handlePasskeyLogin() {
    if (!passkeyLoginEnabled) return
    if (pendingLogin.current) return
    if (requiresLegalConsent && !agreedToLegal) {
      toast.error(legalConsentErrorMessage)
      return
    }

    if (!passkeySupported) {
      toast.error(t('Passkey is not supported on this device'))
      return
    }

    if (!navigator?.credentials) {
      toast.error(t('Passkey is not available in this browser'))
      return
    }

    const controller = new AbortController()
    pendingLogin.current = controller
    const assertOperationCurrent = () => {
      if (controller.signal.aborted || pendingLogin.current !== controller) {
        throw new axios.CanceledError('Passkey login cancelled')
      }
    }
    const assertLoginCurrent = capturePasswordLoginGuard()
    const assertCurrent = () => {
      assertOperationCurrent()
      assertLoginCurrent()
    }
    setIsPasskeyLoading(true)
    try {
      const begin = await beginPasskeyLogin(
        form.getValues('username'),
        assertCurrent
      )
      assertCurrent()
      if (!begin.success) {
        reportAuthError(
          begin,
          t,
          t('Failed to start Passkey login'),
          refreshVerifiedAccountAppeal
        )
        return
      }

      const flowToken = begin.data?.flow_token
      if (!flowToken) {
        throw new Error(t('Login flow expired. Please sign in again.'))
      }
      const finish = await verifyAuthPasskey(
        {
          flowToken,
          options: begin.data?.options ?? begin.data,
          expiresAt: begin.data?.expires_at,
        },
        assertCurrent
      )
      assertOperationCurrent()
      if (!finish) {
        toast.info(t('Passkey login was cancelled'))
        return
      }
      const handled = await completeAuthResponse(finish, {
        alreadyApplied: true,
        signal: controller.signal,
        onMFA: () => {
          assertOperationCurrent()
          redirectTo2FA(redirectTo)
        },
        onBundle: async (bundle) => {
          assertOperationCurrent()
          await handleLoginSuccess(bundle, redirectTo, true, controller.signal)
          assertOperationCurrent()
          toast.success(t('Signed in with Passkey'))
        },
      })
      assertOperationCurrent()
      if (!handled) {
        reportAuthError(
          finish,
          t,
          t('Failed to complete Passkey login'),
          refreshVerifiedAccountAppeal
        )
      }
    } catch (error) {
      if (
        controller.signal.aborted ||
        pendingLogin.current !== controller ||
        axios.isCancel(error)
      ) {
        return
      }
      if (error instanceof DOMException && error.name === 'NotAllowedError') {
        toast.info(t('Passkey login was cancelled or timed out'))
      } else {
        reportAuthError(
          error,
          t,
          t('Passkey login failed'),
          refreshVerifiedAccountAppeal
        )
      }
    } finally {
      if (pendingLogin.current === controller) {
        pendingLogin.current = null
        if (!controller.signal.aborted) setIsPasskeyLoading(false)
      }
    }
  }

  const alternativeLoginMethods = (
    <>
      {/* The configured Google custom OAuth provider is promoted into the
          familiar branded entry point; all other providers stay secondary. */}
      <OAuthProviders
        onVerifiedAccountAppeal={() =>
          setAppealRefreshKey((value) => value + 1)
        }
        status={status}
        registrationCapabilitiesReady={capabilitiesReady}
        redirectTo={redirectTo}
        acceptedLegal={agreedToLegal}
        disabled={
          isLoading ||
          isPasskeyLoading ||
          (requiresLegalConsent && !agreedToLegal)
        }
        onWeChatLogin={hasWeChatLogin ? handleOpenWeChatDialog : undefined}
        isWeChatLoading={isWeChatSubmitting}
        featureGoogle
      />

      {passkeyLoginEnabled && (
        <div className='space-y-1'>
          <Button
            type='button'
            variant='ghost'
            disabled={passkeyButtonDisabled}
            onClick={handlePasskeyLogin}
            className='text-muted-foreground hover:text-foreground h-10 w-full justify-center gap-2 rounded-xl'
          >
            {isPasskeyLoading ? (
              <Loader2 className='h-4 w-4 animate-spin' />
            ) : (
              <KeyRound className='h-4 w-4' />
            )}
            {t('Sign in with Passkey')}
          </Button>
          {passkeySupported && (
            <p className='text-muted-foreground text-xs'>
              {t(
                'Enter your username if your security key does not support account discovery.'
              )}
            </p>
          )}
          {!passkeySupported && (
            <p className='text-muted-foreground text-xs'>
              {t('Passkey is not supported on this device.')}
            </p>
          )}
        </div>
      )}
    </>
  )

  return (
    <Form {...form}>
      <AccountClosureRecovery />
      <form
        onSubmit={(event) => {
          void form.handleSubmit(onSubmit)(event)
        }}
        className={cn('auth-login-form grid gap-6', className)}
        {...props}
      >
        <HumanGrantButton
          configured={
            capabilitiesReady
              ? status?.configured_components?.humangrant
              : false
          }
          disabled={isLoading || (requiresLegalConsent && !agreedToLegal)}
        />
        {hasAlternativeLogin && alternativeLoginMethods}

        {hasAlternativeLogin && passwordLoginEnabled ? (
          <div
            className='auth-login-divider text-muted-foreground py-1 text-center text-xs'
            aria-hidden='true'
          >
            {t('Or')}
          </div>
        ) : null}

        {(passwordLoginEnabled || passkeyLoginEnabled) && (
          <>
            {/* Username Field */}
            <FormField
              control={form.control}
              name='username'
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{t('Username or Email')}</FormLabel>
                  <FormControl>
                    <Input
                      autoComplete='username'
                      autoCapitalize='none'
                      autoCorrect='off'
                      spellCheck={false}
                      placeholder={t('Enter your username or email')}
                      className='auth-field h-11 rounded-xl'
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
          </>
        )}

        {passwordLoginEnabled && (
          <>
            {/* Password Field */}
            <FormField
              control={form.control}
              name='password'
              render={({ field }) => (
                <FormItem>
                  <div className='auth-password-label flex items-center justify-between gap-4'>
                    <FormLabel>{t('Password')}</FormLabel>
                    <Link
                      to='/forgot-password'
                      className='auth-forgot-password text-muted-foreground hover:text-foreground rounded-sm text-sm underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-offset-4'
                    >
                      {t('Forgot password?')}
                    </Link>
                  </div>
                  <FormControl>
                    <PasswordInput
                      autoComplete='current-password'
                      placeholder={t('Enter password')}
                      className='auth-field h-11 rounded-xl'
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            {/* Submit Button */}
            <Button
              type='submit'
              className='auth-submit mt-2 h-12 w-full justify-center gap-2 rounded-xl'
              disabled={isLoading || isPasskeyLoading}
            >
              {isLoading && <Loader2 className='animate-spin' />}
              {t('Sign in')}
            </Button>

            {/* Turnstile */}
            {isTurnstileEnabled && (
              <div className='mt-2'>
                <Turnstile
                  key={turnstileWidgetKey}
                  siteKey={turnstileSiteKey}
                  onVerify={setTurnstileToken}
                  onExpire={resetTurnstile}
                />
              </div>
            )}
          </>
        )}

        {!passwordLoginEnabled && (
          <div className='flex justify-end'>
            <Link
              to='/forgot-password'
              className='auth-forgot-password text-muted-foreground hover:text-foreground rounded-sm text-sm underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-offset-4'
            >
              {t('Forgot password?')}
            </Link>
          </div>
        )}

        <LegalConsent
          status={status}
          checked={agreedToLegal}
          onCheckedChange={setAgreedToLegal}
          className='auth-legal-consent mt-1 border-0 bg-transparent p-0'
        />

        {!hasAlternativeLogin && alternativeLoginMethods}
      </form>

      {hasWeChatLogin && (
        <Dialog
          open={isWeChatDialogOpen}
          onOpenChange={handleWeChatDialogChange}
          title={t('WeChat sign in')}
          description={t(
            'Scan the QR code to follow the official account and reply with “验证码” to receive your verification code.'
          )}
          contentClassName='max-w-sm'
          headerClassName='text-left'
          contentHeight='auto'
          bodyClassName='space-y-4'
          footer={
            <>
              <Button
                type='button'
                variant='outline'
                onClick={() => handleWeChatDialogChange(false)}
                disabled={isWeChatSubmitting}
              >
                {t('Cancel')}
              </Button>
              <Button
                type='button'
                onClick={handleWeChatLogin}
                disabled={
                  isWeChatSubmitting ||
                  !wechatCode.trim() ||
                  (requiresLegalConsent && !agreedToLegal)
                }
                className='gap-2'
              >
                {isWeChatSubmitting ? (
                  <Loader2 className='h-4 w-4 animate-spin' />
                ) : null}
                {t('Confirm')}
              </Button>
            </>
          }
        >
          {wechatQrCodeUrl ? (
            <div className='flex justify-center'>
              <img
                src={wechatQrCodeUrl}
                alt={t('WeChat login QR code')}
                className='h-40 w-40 rounded-md border object-contain'
              />
            </div>
          ) : (
            <p className='text-muted-foreground text-sm'>
              {t('QR code is not configured. Please contact support.')}
            </p>
          )}
          <div className='grid gap-2'>
            <Label htmlFor='wechat-code'>{t('Verification code')}</Label>
            <Input
              id='wechat-code'
              placeholder={t('Enter the verification code')}
              value={wechatCode}
              onChange={(event) => setWeChatCode(event.target.value)}
              autoComplete='one-time-code'
            />
          </div>
        </Dialog>
      )}
      <div className='auth-account-help pt-2 text-center'>
        <AccountAppealForm refreshKey={appealRefreshKey} />
      </div>
    </Form>
  )
}

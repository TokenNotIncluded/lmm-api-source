/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import { requestAssistantOpen } from '@/features/assistant/assistant-events'
import { federatedProviders } from '@/features/auth/lib/federated-oauth'
import {
  hasRegistrationMethod,
  isOAuthProviderConfigured,
  isOAuthRegistrationAllowed,
  isPasswordRegistrationEnabled,
  isRegistrationEnabled,
} from '@/features/auth/lib/registration'
import type { SystemStatus } from '@/features/auth/types'
import { getAccountNextStep } from '@/features/onboarding/next-step'
import { getStatus } from '@/lib/api'
import { getOnboardingState } from '@/lib/console-activation'
import { useAuthStore } from '@/stores/auth-store'

import {
  clip,
  EMPTY_INPUT_SCHEMA,
  ensureNotAborted,
  ensureObject,
  optionalEnum,
  requireSignedIn,
  type ModelContextTool,
  type WebMcpToolFactory,
} from '../tool-kit'

/** Sign-in entry points a browser agent may open. It never submits a form. */
const AUTH_PAGES = [
  '/sign-in',
  '/sign-up',
  '/forgot-password',
  '/otp',
  '/reset',
] as const
type AuthPage = (typeof AUTH_PAGES)[number]

/** Presets an agent may hand to the assistant; free-form text stays a human's job. */
const ASSISTANT_PRESETS = ['onboarding', 'human', 'plan'] as const

const OAUTH_METHODS = [
  'google',
  'github',
  'discord',
  'linuxdo',
  'telegram',
] as const

function readStatus(status: SystemStatus | null): {
  method: string
  label: string
  sign_in: boolean
  register: boolean
}[] {
  const rows = OAUTH_METHODS.map((method) => ({
    method,
    label:
      method === 'github'
        ? 'GitHub'
        : method === 'linuxdo'
          ? 'LinuxDO'
          : method[0].toUpperCase() + method.slice(1),
    sign_in: isOAuthProviderConfigured(status, method),
    register:
      isOAuthProviderConfigured(status, method) &&
      isOAuthRegistrationAllowed(status, method),
  }))
  const federated = federatedProviders(status).map((provider) => ({
    method: `federated:${provider.id}`,
    label: provider.name,
    sign_in: true,
    register: isOAuthRegistrationAllowed(status, provider),
  }))
  return [...rows, ...federated].filter((row) => row.sign_in || row.register)
}

/** Live capabilities only: a cached snapshot must not advertise a disabled path. */
async function readSystemStatus(signal: AbortSignal): Promise<SystemStatus> {
  ensureNotAborted(signal)
  const data = await getStatus()
  ensureNotAborted(signal)
  return data as SystemStatus
}

function onboardingSnapshot() {
  const user = useAuthStore.getState().auth.user
  const state = getOnboardingState(user)
  const next = getAccountNextStep(user)
  return {
    authenticated: Boolean(user),
    stage: state.stage,
    credential_complete: state.credentialComplete,
    first_request_complete: state.firstRequestComplete,
    next_step: { path: next.to, label: next.label },
  }
}

export const authOnboardingTools: WebMcpToolFactory = ({ router }) => {
  const tools: ModelContextTool[] = [
    {
      name: 'lmm_auth_methods',
      title: 'Read available sign-in methods',
      description:
        'Read which sign-in and registration methods the server currently offers: password login, password registration, open registration, and each configured OAuth provider with whether it can sign in and whether it can create an account. Never returns credentials, secrets, client IDs, or tokens.',
      inputSchema: EMPTY_INPUT_SCHEMA,
      annotations: { readOnlyHint: true, untrustedContentHint: true },
      execute: async (_input, options) => {
        const status = await readSystemStatus(options.signal)
        ensureNotAborted(options.signal)
        const providers = readStatus(status)
        return {
          registration_open: isRegistrationEnabled(status),
          // Absent means the server did not restrict it; never advertise a
          // sign-in path as closed on missing data.
          password_login: (status.password_login_enabled ?? true) !== false,
          password_registration: isPasswordRegistrationEnabled(status),
          can_create_account: hasRegistrationMethod(status),
          providers,
          sign_in_path: '/sign-in',
          sign_up_path: '/sign-up',
          note: 'A method is listed only when the server reports it configured. Opening a sign-in page never signs anyone in.',
        }
      },
    },
    {
      name: 'lmm_auth_open',
      title: 'Open a sign-in or account-recovery page',
      description:
        'Open one of the authentication pages in the current tab: sign in, create an account, forgot password, one-time code, or password reset. This only navigates. It never fills a form, submits a password or verification code, or signs in or registers on the user’s behalf — the human completes the form.',
      inputSchema: {
        type: 'object',
        properties: {
          page: { type: 'string', enum: AUTH_PAGES },
        },
        required: ['page'],
        additionalProperties: false,
      },
      annotations: { readOnlyHint: true },
      execute: async (rawInput, options) => {
        const input = ensureObject(rawInput)
        const page = optionalEnum(input, 'page', AUTH_PAGES)
        if (!page) throw new TypeError('page is required')
        ensureNotAborted(options.signal)
        await router.navigate({ to: page as AuthPage })
        return { path: page, opened: true }
      },
    },
    {
      name: 'lmm_onboarding_status',
      title: 'Read onboarding stage and next step',
      description:
        'Read the current account setup state: credential and first-request progress and a recommended next step. Works while signed out. Never returns credentials or private profile data.',
      inputSchema: EMPTY_INPUT_SCHEMA,
      annotations: { readOnlyHint: true },
      execute: async (_input, options) => {
        ensureNotAborted(options.signal)
        return onboardingSnapshot()
      },
    },
    {
      name: 'lmm_onboarding_open_assistant',
      title: 'Open onboarding help',
      description:
        'Open setup help in the console assistant for a signed-in account. This tool sends no message.',
      inputSchema: {
        type: 'object',
        properties: {
          preset: { type: 'string', enum: ASSISTANT_PRESETS },
        },
        required: ['preset'],
        additionalProperties: false,
      },
      annotations: { readOnlyHint: true },
      execute: async (rawInput, options) => {
        requireSignedIn()
        const input = ensureObject(rawInput)
        const preset = optionalEnum(input, 'preset', ASSISTANT_PRESETS)
        if (!preset) throw new TypeError('preset is required')
        ensureNotAborted(options.signal)
        requestAssistantOpen(preset)
        return { preset, opened: true }
      },
    },
    {
      name: 'lmm_setup_status',
      title: 'Read first-install setup status',
      description:
        'Read the public first-install setup flag, version and system name. It never reads or writes setup credentials, and it cannot complete setup for anyone; the operator fills that form by hand.',
      inputSchema: EMPTY_INPUT_SCHEMA,
      annotations: { readOnlyHint: true, untrustedContentHint: true },
      execute: async (_input, options) => {
        const status = await readSystemStatus(options.signal)
        ensureNotAborted(options.signal)
        const setup = status.setup
        return {
          setup_required: typeof setup === 'boolean' ? !setup : null,
          setup_path: '/setup',
          version: clip(status.version, 40),
          system_name: clip(status.system_name, 80),
        }
      },
    },
  ]
  return tools
}

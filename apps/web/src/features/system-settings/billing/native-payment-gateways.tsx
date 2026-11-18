/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'

import { NativeEPaySettings } from './native-epay-settings'
import { NativePaymentSettings } from './native-payment-settings'

export function NativePaymentGateways() {
  const { t } = useTranslation()
  return (
    <Tabs defaultValue='pancake' className='gap-8'>
      <TabsList variant='navigation' aria-label={t('Payment Gateway')}>
        <TabsTrigger value='pancake'>{t('Waffo Pancake')}</TabsTrigger>
        <TabsTrigger value='epay'>{t('Epay Gateway')}</TabsTrigger>
      </TabsList>
      <TabsContent value='pancake' keepMounted>
        <NativePaymentSettings />
      </TabsContent>
      <TabsContent value='epay' keepMounted>
        <NativeEPaySettings />
      </TabsContent>
    </Tabs>
  )
}

/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { NativePaymentSettings } from '@/features/system-settings/billing/native-payment-settings'

import { PaymentProviderStatus } from './provider-status'

export function PaymentConfiguration() {
  return (
    <div className='space-y-8'>
      <PaymentProviderStatus />
      <NativePaymentSettings />
    </div>
  )
}

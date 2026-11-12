/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { MAX_IMPORT_BYTES, MAX_IMPORT_ITEMS, safeStoreUrl } from './utils'

const copy = {
  redemption: 'Redemption codes',
  license: 'License keys',
  download: 'Download links',
  account: 'Account details',
  cardHelp:
    'For activation keys. Import one key per line; buyers copy each key exactly as supplied.',
  textHelp:
    'For short text items. Import one item per line; buyers receive the original text.',
  customHelp:
    'For multi-line notes or instructions. Add each complete item separately; line breaks stay within that item.',
  redemptionHelp:
    'For codes redeemed on another site. Add a code, an optional redemption URL, and instructions for each item.',
  licenseHelp:
    'For software licenses. Add the license key, product name, and activation instructions for each item.',
  downloadHelp:
    'For downloadable content. Add a download URL, an optional access code, and instructions for each item.',
  accountHelp:
    'For account delivery. Add the username, password, optional login URL, and instructions together as one item.',
  add: 'Add one item',
  remove: 'Remove this item',
  contents: 'Complete item contents',
  redeemCode: 'Redemption code',
  redeemUrl: 'Redemption URL',
  licenseKey: 'License key',
  downloadUrl: 'Download URL',
  accessCode: 'Access code',
  loginUrl: 'Login URL',
  instructions: 'Instructions',
  showPassword: 'Show delivered password',
  hidePassword: 'Hide delivered password',
  openDownload: 'Open download link',
  openRedemption: 'Open redemption site',
  openLogin: 'Open login page',
  missing: 'Complete the required fields before adding this item.',
} as const

const fixedCopy = {
  template: 'Fixed content',
  content: 'Delivery content',
  unlimited: 'Unlimited supply',
  help: 'Every purchase receives the same private content. No inventory import is needed. Sales and purchase limits still apply.',
  privateHelp:
    'Only the seller and eligible paid buyers can read this content. Existing orders keep the content saved at purchase.',
  invalid: 'Enter delivery content within 16,000 bytes.',
  noImport:
    'This variant uses fixed content and does not accept inventory imports. Edit its delivery content in the product or variant editor.',
  draftHelp:
    'Save a draft first. Submit it after adding delivery content and configuring payment methods.',
} as const

export {
  copy as STORE_DELIVERY_TEMPLATE_COPY,
  fixedCopy as STORE_FIXED_CONTENT_COPY,
}

export type StoreDeliveryTemplate =
  | 'card-key'
  | 'text'
  | 'custom-text'
  | 'fixed-content'
  | 'redemption-code'
  | 'license-key'
  | 'download-link'
  | 'account-details'
export type StructuredDeliveryTemplate =
  | 'redemption-code'
  | 'license-key'
  | 'download-link'
  | 'account-details'
type DeliveryField = {
  name: string
  label: string
  required?: boolean
  url?: boolean
  password?: boolean
}
export const DELIVERY_TEMPLATES: Record<
  StoreDeliveryTemplate,
  { label: string; help: string }
> = {
  'card-key': { label: 'Activation keys', help: copy.cardHelp },
  text: { label: 'Text items', help: copy.textHelp },
  'custom-text': { label: 'Custom text', help: copy.customHelp },
  'fixed-content': { label: fixedCopy.template, help: fixedCopy.help },
  'redemption-code': { label: copy.redemption, help: copy.redemptionHelp },
  'license-key': { label: copy.license, help: copy.licenseHelp },
  'download-link': { label: copy.download, help: copy.downloadHelp },
  'account-details': { label: copy.account, help: copy.accountHelp },
}
export const DELIVERY_FIELDS: Record<
  StructuredDeliveryTemplate,
  DeliveryField[]
> = {
  'redemption-code': [
    { name: 'code', label: copy.redeemCode, required: true },
    { name: 'redeem_url', label: copy.redeemUrl, url: true },
    { name: 'instructions', label: copy.instructions },
  ],
  'license-key': [
    { name: 'license_key', label: copy.licenseKey, required: true },
    { name: 'product', label: 'Product' },
    { name: 'instructions', label: copy.instructions },
  ],
  'download-link': [
    { name: 'url', label: copy.downloadUrl, required: true, url: true },
    { name: 'access_code', label: copy.accessCode },
    { name: 'instructions', label: copy.instructions },
  ],
  'account-details': [
    { name: 'username', label: 'Username', required: true },
    { name: 'password', label: 'Password', required: true, password: true },
    { name: 'url', label: copy.loginUrl, url: true },
    { name: 'instructions', label: copy.instructions },
  ],
}
export const DELIVERY_FIELD_BYTE_LIMITS: Record<string, number> = {
  code: 4096,
  license_key: 4096,
  password: 4096,
  access_code: 4096,
  username: 1000,
  product: 1000,
  instructions: 16384,
  url: 2048,
  redeem_url: 2048,
}
export function structuredTemplate(
  template: string
): template is StructuredDeliveryTemplate {
  return Object.hasOwn(DELIVERY_FIELDS, template)
}
export function validDeliveryUrl(value: string) {
  return (
    !/\p{Cc}/u.test(value) &&
    value === value.trim() &&
    /^https?:\/\/[^/?#]+/i.test(value) &&
    Boolean(safeStoreUrl(value))
  )
}
export function validDeliveryFields(
  template: StructuredDeliveryTemplate,
  fields: Record<string, unknown>
) {
  const definitions = DELIVERY_FIELDS[template]
  return (
    Object.keys(fields).every((key) =>
      definitions.some((field) => field.name === key)
    ) &&
    definitions.every((field) => {
      const value = fields[field.name]
      if (value === undefined) return !field.required
      if (typeof value !== 'string') return false
      if (field.required && !value.trim()) return false
      if (
        new TextEncoder().encode(value).length >
        DELIVERY_FIELD_BYTE_LIMITS[field.name]
      ) {
        return false
      }
      return !field.url || !value || validDeliveryUrl(value)
    })
  )
}
export function encodeDeliveryItem(
  template: StructuredDeliveryTemplate,
  fields: Record<string, string>
) {
  if (!validDeliveryFields(template, fields)) return undefined
  return JSON.stringify({
    lmm_store_delivery: 1,
    template,
    fields: Object.fromEntries(
      Object.entries(fields).filter(([, value]) => value !== '')
    ),
  })
}
export function parseDeliveryItem(
  raw: string,
  frozenTemplate: string | undefined
):
  | {
      template: StructuredDeliveryTemplate
      fields: Record<string, string>
    }
  | undefined {
  if (!frozenTemplate || !structuredTemplate(frozenTemplate)) return undefined
  try {
    const payload = JSON.parse(raw)
    if (
      !payload ||
      typeof payload !== 'object' ||
      Array.isArray(payload) ||
      Object.keys(payload).some(
        (key) => !['lmm_store_delivery', 'template', 'fields'].includes(key)
      ) ||
      payload.lmm_store_delivery !== 1 ||
      payload.template !== frozenTemplate ||
      !payload.fields ||
      typeof payload.fields !== 'object' ||
      Array.isArray(payload.fields) ||
      !validDeliveryFields(frozenTemplate, payload.fields)
    ) {
      return undefined
    }
    return { template: frozenTemplate, fields: payload.fields }
  } catch {
    return undefined
  }
}
export function deliveryItemText(
  raw: string,
  template: string | undefined,
  translate: (key: string) => string
) {
  const parsed = parseDeliveryItem(raw, template)
  if (!parsed) return raw
  return DELIVERY_FIELDS[parsed.template]
    .filter((field) => parsed.fields[field.name])
    .map((field) => `${translate(field.label)}: ${parsed.fields[field.name]}`)
    .join('\n')
}

export function validateComposedItems(items: string[]) {
  if (
    items.length > MAX_IMPORT_ITEMS ||
    items.some((item) => new TextEncoder().encode(item).length > 32768) ||
    new TextEncoder().encode(JSON.stringify({ items })).length >
      MAX_IMPORT_BYTES
  ) {
    throw new Error('Inventory import is too large')
  }
}

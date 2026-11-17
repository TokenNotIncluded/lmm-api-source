/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  hasNativeDisplayNameControls,
  nativeProfileIdSchema,
} from './profile-contract'

const encoder = new TextEncoder()
const unicode = (value: string) => !/[\ud800-\udfff]/u.test(value)
const group = z
  .string()
  .refine(
    (value) =>
      unicode(value) &&
      value !== 'auto' &&
      [...value].length >= 1 &&
      [...value].length <= 64 &&
      encoder.encode(value).length <= 256 &&
      !hasNativeDisplayNameControls(value)
  )

function compareGroups(left: string, right: string): number {
  const a = encoder.encode(left)
  const b = encoder.encode(right)
  for (let index = 0; index < Math.min(a.length, b.length); index++) {
    if (a[index] !== b[index]) return a[index] - b[index]
  }
  return a.length - b.length
}

export const userAccessValuesSchema = z
  .object({
    groups: z
      .array(group)
      .max(100)
      .refine((values) =>
        values.every(
          (value, index) =>
            index === 0 || compareGroups(values[index - 1], value) < 0
        )
      ),
  })
  .strict()

export const userAccessSnapshotSchema = userAccessValuesSchema
  .extend({
    contract: z.literal('core.admin-user-access.v1'),
    user_id: nativeProfileIdSchema,
    auth_version: nativeProfileIdSchema,
    active: z.boolean(),
    account_active: z.boolean(),
  })
  .strict()

export const userAccessUpdateSchema = z
  .object({ changed: z.boolean(), user: userAccessSnapshotSchema })
  .strict()

export type UserAccessValues = z.infer<typeof userAccessValuesSchema>
export type UserAccessSnapshot = z.infer<typeof userAccessSnapshotSchema>
export type UserAccessUpdate = z.infer<typeof userAccessUpdateSchema>

export function accessValues(snapshot: UserAccessValues): UserAccessValues {
  return {
    groups: [...snapshot.groups],
  }
}

export function parseAccessGroups(value: string): string[] {
  // Spaces are part of a group name. Only empty lines are omitted.
  return value
    .split(/\r?\n/)
    .filter((line) => line !== '')
    .sort(compareGroups)
}

export function sameAccess(left: UserAccessValues, right: UserAccessValues) {
  return (
    left.groups.length === right.groups.length &&
    left.groups.every((value, index) => value === right.groups[index])
  )
}

export function validAccessReason(value: string): boolean {
  return (
    unicode(value) &&
    value.trim().length > 0 &&
    encoder.encode(value).length <= 1000 &&
    !hasNativeDisplayNameControls(value)
  )
}

export function matchesAccessUpdate(
  update: UserAccessUpdate,
  before: UserAccessSnapshot,
  values: UserAccessValues
): boolean {
  const changed = !sameAccess(before, values)
  return (
    update.changed === changed &&
    update.user.user_id === before.user_id &&
    update.user.active === before.active &&
    update.user.account_active === before.account_active &&
    (changed
      ? BigInt(update.user.auth_version) > BigInt(before.auth_version)
      : update.user.auth_version === before.auth_version) &&
    sameAccess(update.user, values)
  )
}

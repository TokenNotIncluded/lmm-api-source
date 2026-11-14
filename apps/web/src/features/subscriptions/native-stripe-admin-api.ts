/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from "zod";

import {
  CommerceFailure,
  commerceRequest,
  currentCommerceScope,
  nativeId,
  operationKey,
  type CommerceScope,
} from "@/features/commerce/native-client";
import { nativeChannelSchema } from "@/features/wallet/lib/native-checkout";
import { ROLE } from "@/lib/roles";
import { useAuthStore } from "@/stores/auth-store";

import { planSchema } from "./native-api";
import { stripePriceSchema } from "./native-stripe-api";

const writablePriceSchema = stripePriceSchema;
export const saveStripePriceSchema = z
  .object({ operation_key: operationKey, price: writablePriceSchema })
  .strict();
export type SaveStripePrice = z.infer<typeof saveStripePriceSchema>;
export const pendingStripePriceSchema = z
  .object({ plan: planSchema, request: saveStripePriceSchema })
  .strict()
  .refine(
    (value) =>
      value.plan.id === value.request.price.plan_id &&
      value.plan.revision === value.request.price.plan_revision,
  );
export type PendingStripePrice = z.infer<typeof pendingStripePriceSchema>;
const stripeChannelSchema = z.object({
  id: z
    .string()
    .regex(/^[1-9][0-9]*$/)
    .transform(Number)
    .pipe(nativeId),
  provider: z.literal("stripe"),
  currency: z.literal("USD"),
  environment: z.enum(["test", "live"]),
  payment_type: z.string().optional(),
});
export type StripeAdminChannel = z.infer<typeof stripeChannelSchema>;
function assertAdmin(scope: CommerceScope) {
  if (!currentCommerceScope(scope))
    throw new CommerceFailure("SESSION_CHANGED");
  if ((useAuthStore.getState().auth.user?.role ?? 0) < ROLE.SUPER_ADMIN) {
    throw new CommerceFailure("FORBIDDEN", false);
  }
}
export async function listAdminStripePrices(
  scope: CommerceScope,
  planId: number,
) {
  assertAdmin(scope);
  nativeId.parse(planId);
  const result = await commerceRequest(
    scope,
    "get",
    `/core/v1/subscriptions/admin/plans/${planId}/stripe-prices`,
    z.object({ prices: z.array(stripePriceSchema) }),
  );
  assertAdmin(scope);
  if (result.prices.some((price) => price.plan_id !== planId)) {
    throw new CommerceFailure("PRICE_MISMATCH");
  }
  return result.prices;
}
export async function listAdminStripeChannels(scope: CommerceScope) {
  assertAdmin(scope);
  const result = await commerceRequest(
    scope,
    "get",
    "/api/user/payments/channels",
    z.object({
      success: z.literal(true),
      data: z.object({
        available: z.boolean(),
        channels: z.array(nativeChannelSchema),
      }),
    }),
  );
  assertAdmin(scope);
  if (!result.data.available) return [];
  return result.data.channels
    .filter(
      (channel) => channel.provider === "stripe" && channel.currency === "USD",
    )
    .map((channel) => stripeChannelSchema.parse(channel));
}
export async function saveAdminStripePrice(
  scope: CommerceScope,
  input: SaveStripePrice,
) {
  assertAdmin(scope);
  const request = saveStripePriceSchema.parse(input);
  const result = await commerceRequest(
    scope,
    "post",
    `/core/v1/subscriptions/admin/plans/${request.price.plan_id}/stripe-prices`,
    writablePriceSchema,
    {
      ...request,
      price: {
        ...request.price,
        plan_id: String(request.price.plan_id),
        plan_revision: String(request.price.plan_revision),
        channel_id: String(request.price.channel_id),
        interval_count: String(request.price.interval_count),
      },
    },
  );
  assertAdmin(scope);
  if (JSON.stringify(result) !== JSON.stringify(request.price)) {
    throw new CommerceFailure("PRICE_MISMATCH");
  }
  return result;
}

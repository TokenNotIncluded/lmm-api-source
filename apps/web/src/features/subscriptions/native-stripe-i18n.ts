/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { i18n } from "i18next";

export const stripeTranslations = {
  en: {
    "Billing interval": "Billing interval",
    "Interval count": "Interval count",
    "Return after payment": "Return after payment",
    "Return after cancellation": "Return after cancellation",
    "Only verified paid invoices grant subscription access.":
      "Only verified paid invoices grant subscription access.",
    "Past due": "Past due",
    Unpaid: "Unpaid",
    Trialing: "Trialing",
    Paused: "Paused",
    "Incomplete expired": "Incomplete expired",
  },
  "zh-CN": {
    "Billing interval": "账单周期",
    "Interval count": "周期数量",
    "Return after payment": "付款后返回地址",
    "Return after cancellation": "取消后返回地址",
    "Only verified paid invoices grant subscription access.":
      "只有经核验的已付款账单才能发放订阅权益。",
    "Past due": "已逾期",
    Unpaid: "未付款",
    Trialing: "试用中",
    Paused: "已暂停",
    "Incomplete expired": "未完成且已过期",
  },
  "zh-TW": {
    "Billing interval": "帳單週期",
    "Interval count": "週期數量",
    "Return after payment": "付款後返回網址",
    "Return after cancellation": "取消後返回網址",
    "Only verified paid invoices grant subscription access.":
      "只有經核驗的已付款帳單才能發放訂閱權益。",
    "Past due": "已逾期",
    Unpaid: "未付款",
    Trialing: "試用中",
    Paused: "已暫停",
    "Incomplete expired": "未完成且已過期",
  },
  fr: {
    "Billing interval": "Périodicité de facturation",
    "Interval count": "Nombre de périodes",
    "Return after payment": "Retour après paiement",
    "Return after cancellation": "Retour après annulation",
    "Only verified paid invoices grant subscription access.":
      "Seules les factures payées et vérifiées donnent accès à l’abonnement.",
    "Past due": "En retard",
    Unpaid: "Impayé",
    Trialing: "En période d’essai",
    Paused: "En pause",
    "Incomplete expired": "Incomplet et expiré",
  },
  ru: {
    "Billing interval": "Период оплаты",
    "Interval count": "Количество периодов",
    "Return after payment": "Возврат после оплаты",
    "Return after cancellation": "Возврат после отмены",
    "Only verified paid invoices grant subscription access.":
      "Доступ по подписке предоставляется только по проверенным оплаченным счетам.",
    "Past due": "Просрочено",
    Unpaid: "Не оплачено",
    Trialing: "Пробный период",
    Paused: "Приостановлено",
    "Incomplete expired": "Не завершено и истекло",
  },
  ja: {
    "Billing interval": "請求周期",
    "Interval count": "周期数",
    "Return after payment": "支払い後の戻り先",
    "Return after cancellation": "キャンセル後の戻り先",
    "Only verified paid invoices grant subscription access.":
      "支払い済みの請求書を検証した後にのみ、サブスクリプションの利用権が付与されます。",
    "Past due": "支払い遅延",
    Unpaid: "未払い",
    Trialing: "試用中",
    Paused: "一時停止中",
    "Incomplete expired": "未完了のまま期限切れ",
  },
  vi: {
    "Billing interval": "Chu kỳ thanh toán",
    "Interval count": "Số chu kỳ",
    "Return after payment": "Quay lại sau khi thanh toán",
    "Return after cancellation": "Quay lại sau khi hủy",
    "Only verified paid invoices grant subscription access.":
      "Chỉ hóa đơn đã thanh toán và được xác minh mới cấp quyền sử dụng gói đăng ký.",
    "Past due": "Quá hạn",
    Unpaid: "Chưa thanh toán",
    Trialing: "Đang dùng thử",
    Paused: "Đã tạm dừng",
    "Incomplete expired": "Chưa hoàn tất và đã hết hạn",
  },
};

const registered = new WeakSet<i18n>();
export function registerStripeTranslations(instance: i18n) {
  if (registered.has(instance)) return;
  for (const [language, resource] of Object.entries(stripeTranslations))
    instance.addResourceBundle(language, "translation", resource, true, false);
  registered.add(instance);
}

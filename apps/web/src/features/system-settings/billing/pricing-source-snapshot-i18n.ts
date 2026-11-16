/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { i18n } from 'i18next'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'

const namespace = 'pricing-source-snapshot'
const languages = ['en', 'zh-CN', 'zh-TW', 'fr', 'ja', 'ru', 'vi'] as const
const rows = [
  [
    'Original configuration',
    '原始配置',
    '原始設定',
    'Configuration d’origine',
    '元の設定',
    'Исходная конфигурация',
    'Cấu hình gốc',
  ],
  [
    'Read-only production snapshot. Only published native prices apply to requests.',
    '生产配置只读快照。请求仅按已发布的原生价格计费。',
    '正式環境設定唯讀快照。請求僅依已發布的原生價格計費。',
    'Instantané de production en lecture seule. Seuls les tarifs natifs publiés s’appliquent aux requêtes.',
    '本番設定の読み取り専用スナップショットです。リクエストには公開済みのネイティブ料金のみが適用されます。',
    'Снимок рабочей конфигурации только для чтения. К запросам применяются лишь опубликованные нативные тарифы.',
    'Bản chụp cấu hình sản xuất chỉ đọc. Yêu cầu chỉ áp dụng giá gốc đã công bố.',
  ],
  [
    'Captured at',
    '采集时间',
    '擷取時間',
    'Date de capture',
    '取得日時',
    'Время снимка',
    'Thời điểm chụp',
  ],
  [
    'Pricing options',
    '价格配置项',
    '價格設定項',
    'Options tarifaires',
    '料金設定項目',
    'Настройки тарифов',
    'Tùy chọn giá',
  ],
  [
    'Enabled source channels',
    '来源启用渠道',
    '來源啟用通道',
    'Canaux source actifs',
    '有効なソースチャネル',
    'Активные исходные каналы',
    'Kênh nguồn đang bật',
  ],
  [
    'Unconverted model/group pairs',
    '未转换的模型与分组',
    '未轉換的模型與群組',
    'Paires modèle/groupe non converties',
    '未変換のモデルとグループ',
    'Непреобразованные пары модель/группа',
    'Cặp mô hình/nhóm chưa chuyển đổi',
  ],
  [
    'These source entries were not converted by this import. Check the published native rules before use.',
    '本次导入未转换这些来源条目。使用前请核对已发布的原生规则。',
    '本次匯入未轉換這些來源項目。使用前請核對已發布的原生規則。',
    'Ces entrées source n’ont pas été converties lors de cet import. Vérifiez les règles natives publiées avant utilisation.',
    '今回のインポートではこれらの元の項目は変換されていません。利用前に公開済みのネイティブルールをご確認ください。',
    'Эти исходные записи не были преобразованы при этом импорте. Перед использованием проверьте опубликованные нативные правила.',
    'Các mục nguồn này chưa được chuyển đổi trong lần nhập này. Kiểm tra quy tắc gốc đã công bố trước khi dùng.',
  ],
  [
    'Search original configuration',
    '搜索原始配置',
    '搜尋原始設定',
    'Rechercher dans la configuration d’origine',
    '元の設定を検索',
    'Поиск по исходной конфигурации',
    'Tìm trong cấu hình gốc',
  ],
  [
    'Download complete JSON',
    '下载完整 JSON',
    '下載完整 JSON',
    'Télécharger le JSON complet',
    '完全な JSON をダウンロード',
    'Скачать полный JSON',
    'Tải JSON đầy đủ',
  ],
  [
    'Original pricing values',
    '原始价格值',
    '原始價格值',
    'Valeurs tarifaires d’origine',
    '元の料金値',
    'Исходные значения тарифов',
    'Giá trị giá gốc',
  ],
  [
    'Source model and group mappings',
    '来源模型与分组映射',
    '來源模型與群組對應',
    'Correspondances source des modèles et groupes',
    '元のモデルとグループの対応',
    'Исходные соответствия моделей и групп',
    'Ánh xạ mô hình và nhóm nguồn',
  ],
  [
    'Conversion reason',
    '未转换原因',
    '未轉換原因',
    'Motif de non-conversion',
    '未変換の理由',
    'Причина отсутствия преобразования',
    'Lý do chưa chuyển đổi',
  ],
  [
    'No original pricing snapshot is available.',
    '尚无原始价格快照。',
    '尚無原始價格快照。',
    'Aucun instantané tarifaire d’origine disponible.',
    '元の料金スナップショットはありません。',
    'Исходный снимок тарифов пока недоступен.',
    'Chưa có bản chụp giá gốc.',
  ],
  [
    'The original pricing snapshot could not be loaded.',
    '无法加载原始价格快照。',
    '無法載入原始價格快照。',
    'Impossible de charger l’instantané tarifaire d’origine.',
    '元の料金スナップショットを読み込めませんでした。',
    'Не удалось загрузить исходный снимок тарифов.',
    'Không tải được bản chụp giá gốc.',
  ],
  [
    'No matching entries',
    '没有匹配条目',
    '沒有符合項目',
    'Aucune entrée correspondante',
    '一致する項目がありません',
    'Совпадений нет',
    'Không có mục phù hợp',
  ],
  [
    'Source SHA-256',
    '来源 SHA-256',
    '來源 SHA-256',
    'SHA-256 de la source',
    'ソース SHA-256',
    'SHA-256 источника',
    'SHA-256 nguồn',
  ],
] as const

const registered = new WeakSet<i18n>()
export function registerPricingSourceTranslations(instance: i18n): void {
  if (registered.has(instance)) return
  for (const [index, language] of languages.entries()) {
    instance.addResourceBundle(
      language,
      namespace,
      Object.fromEntries(rows.map((values) => [values[0], values[index]])),
      true,
      false
    )
  }
  registered.add(instance)
}

export function usePricingSourceTranslation() {
  const translation = useTranslation()
  const { i18n } = translation
  registerPricingSourceTranslations(i18n)
  const language = i18n.resolvedLanguage || i18n.language || 'en'
  const t = useMemo(
    () => i18n.getFixedT(language, [namespace, 'translation']),
    [i18n, language]
  )
  return { ...translation, t }
}

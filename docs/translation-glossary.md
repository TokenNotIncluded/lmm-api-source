# Translation Glossary

Use one terminology table for documentation and UI. Display names and navigation are defined in [product terminology](product.md#terminology); route and permission identifiers do not change with translated labels.

## Shared terminology

| English term | Meaning | French | Russian |
| --- | --- | --- | --- |
| Ratio / Multiplier | Price multiplier | Ratio | Коэффициент |
| Token | API credential, text unit, or access token; qualify by context | Jeton | Токен |
| Channel | Provider access and routing entry | Canal | Канал |
| Group | User or API-key classification affecting quota and pricing | Groupe | Группа |
| Quota | Available service entitlement; Credit where the context requires | Quota / Crédit | Квота / Кредит |
| Prompt | User interaction with a model | Invite | Prompt / Ввод; conversational UI: Промпт |
| Completion | Model output content | Complétion | Вывод |
| Input | Request data, billing or processing input | Entrée | Ввод |
| Output | Returned data, including structured output | Sortie | Вывод |
| Model Ratio | Model pricing multiplier | Ratio du modèle | Коэффициент модели |
| Completion Ratio | Separate output price multiplier | Ratio de complétion | Коэффициент вывода |
| Price per call | Fixed charge per request | Prix fixe | Цена за запрос |
| Pay-as-you-go | Usage-based billing | Paiement à l'utilisation | Оплата по объему |
| Pay-per-call / Pay-per-view | Per-invocation billing; Pay-per-view is legacy wording | Paiement par appel | Оплата за вызов |
| Root User | Highest-privilege operator | Super-administrateur | Суперадминистратор |
| Admin User | Administrator of users, policy and settings | Administrateur | Администратор |
| Normal User | Account without elevated privileges | Utilisateur normal | Обычный пользователь |
| Top Up | Add account quota or balance | Recharge | Пополнение |
| Redemption Code | Code redeemable for quota or a promotion | Code d'échange | Код обмена |
| API Key / Key | Credential used for API authentication | Clé API | API ключ |
| Priority | Preferred channel routing order | Priorité | Приоритет |
| Weight | Load-balancing distribution weight | Poids | Вес |
| Proxy | Proxy server address; protocol/host rules apply | Proxy | Прокси |
| Model Mapping | Rewrite the request model name | Redirection de modèle | Перенаправление модели |
| Provider / Vendor | Organization or service exposing models/APIs | Fournisseur | Поставщик |
| Two-Factor Authentication | Additional account verification | Authentification à deux facteurs | Двухфакторная аутентификация |
| 2FA | Keep the uppercase abbreviation | 2FA | 2FA |

## Context rules

- Distinguish API Token, Text Token and Access Token. Use API Key for an API credential in UI rather than an ambiguous Token label.
- Use Prompt for conversational UI and Input for billing, technical documentation and processing. Output includes both completion content and structured responses.
- Quota describes service entitlement; Credit is an alternative only when its monetary or entitlement context fits.
- Preserve only emojis present in the source. Technical English may remain when used in the source or common in the target language, such as API.

## French

- Pricing uses **Ratio**, not Multiplicateur. Model output uses **Complétion**, not achèvement or finalisation.
- Use **Invite** for prompt interaction and **Entrée** for technical or billing input.
- Prefer **Clé API** and **Code d'échange**; the latter replaces code de redemption. Provider is **Fournisseur**.
- Keep localization plural forms `_one` and `_other`; respect gender and grammar, including masculine `modèle` and `canal`.

## Russian

- Pricing uses **Коэффициент**, not Множитель. Model output uses **Вывод**, not дополнение or завершение.
- Use **Промпт** for conversational UI and **Ввод** for processing, billing and calculation; legacy prompt labels may retain Prompt / Ввод.
- Prefer **API ключ**, **Код обмена** and **Поставщик** for API credentials, redemption codes and providers.
- Keep localization plural forms `_one`, `_few`, `_many`, `_other`; preserve case and grammatical gender, including feminine `модель` and masculine `канал`.

Terminology corrections belong in this table and the affected localization, with the context explained in the Issue or PR.

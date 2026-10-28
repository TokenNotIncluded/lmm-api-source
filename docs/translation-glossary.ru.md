# Russian Glossary (Project Terminology)

This glossary defines standard Russian terms used across project localization.

## Core Concepts

- Emojis are allowed only when present in the source text.
- Purely technical terms are allowed when they appear in the source.
- English technical terms are allowed when common in Russian technical usage (for example, API).

| English Term | Russian Term | Canonical English | Description |
|--------------|--------------|-------------------|-------------|
| Ratio | Коэффициент | Ratio/Multiplier | Multiplier used in price calculation. **Important:** For pricing, always use **Коэффициент** instead of **Множитель**. |
| Token | Токен | Token | API credentials or text units processed by models. |
| Channel | Канал | Channel | API provider access channel. |
| Group | Группа | Group | Classification of users or tokens. |
| Quota | Квота | Quota | Available service quota for the user. |

## Model Related

| English Term | Russian Term | Canonical English | Description |
|--------------|--------------|-------------------|-------------|
| Prompt | Prompt / Ввод | Prompt | Input content for model. |
| Completion | Вывод | Completion | Model output. **Important:** Do not translate as alternatives like "дополнение" or "завершение". |
| Input | Ввод | Input/Prompt | Content sent to the model. |
| Output | Вывод | Output/Completion | Content returned by the model. |
| Model Ratio | Коэффициент модели | Model Ratio | Pricing ratio for different models. |
| Completion Ratio | Коэффициент вывода | Completion Ratio | Additional multiplier for output pricing. |
| Price per call | Цена за запрос | Price per call | Price charged per API call. |
| Pay-as-you-go | Оплата по объему | Pay-as-you-go | Usage-based pricing. |
| Pay-per-view | Оплата за вызов | Pay-per-call | Flat price per call. |

## User Management

| English Term | Russian Term | Canonical English | Description |
|--------------|--------------|-------------------|-------------|
| Root User | Суперадминистратор | Root User | Administrator with highest privileges. |
| Admin User | Администратор | Admin User | System administrator. |
| Normal User | Обычный пользователь | Normal User | User with standard permissions. |

## Recharge & Redemption

| English Term | Russian Term | Canonical English | Description |
|--------------|--------------|-------------------|-------------|
| Top Up | Пополнение | Top Up | Add quota to an account. |
| Redemption Code | Код обмена | Redemption Code | Code redeemable for quota. |

## Channel Management

| English Term | Russian Term | Canonical English | Description |
|--------------|--------------|-------------------|-------------|
| Channel | Канал | Channel | API access channel. |
| API Key | API ключ | API Key | API access key. **Important:** Prefer **API ключ** over alternative wording. |
| Priority | Приоритет | Priority | Channel selection priority. |
| Weight | Вес | Weight | Load-balancing weight. |
| Proxy | Прокси | Proxy | Proxy server address. |
| Model Mapping | Перенаправление модели | Model Mapping | Replace model name in request payload. |
| Provider | Поставщик | Provider/Vendor | Service or API provider. |

## Security Related

| English Term | Russian Term | Canonical English | Description |
|--------------|--------------|-------------------|-------------|
| Two-Factor Authentication | Двухфакторная аутентификация | Two-Factor Authentication | Additional security verification method. |
| 2FA | 2FA | Two-Factor Authentication | Abbreviation. |

## Translation Guidance

### Prompt/Input Context

- **Промпт**: user interaction, chat/LLM context.
- **Ввод**: technical processing, billing, and calculation context.
- **Rule:** Use **Промпт** for conversational UI, **Ввод** for backend and pipeline descriptions.

### Token Context

- API token (API Token)
- Text token processed by model (Text Token)
- System access token (Access Token)

### Quota Notes

- User service quota.
- Sometimes translated as **Кредит**.

### Language Notes

- Correct plural forms are required for localization files (`_one`, `_few`, `_many`, `_other`).
- Watch case/declension in technical terms.
- Respect grammatical gender (for example, `модель` is feminine, `канал` is masculine).

### Standardized Terms

- **Completion / Вывод**: model output.
- **Ratio / Коэффициент**: price multiplier.
- **Redemption Code / Код обмена**: canonical phrasing for top-up code.
- **Provider / Поставщик**: organization or service exposing APIs or models.

---

**Contributor note:** If you spot inconsistent terminology or have better Russian translations, open an Issue or Pull Request.

# French Glossary (Project Terminology)

This glossary defines preferred French translations used across the project documentation and UI.

## Core Concepts

- Emojis are allowed only if they already exist in the source text.
- Technical terms may remain in English when the original text uses them.
- English technical terms are allowed when widely used in French-speaking technical environments (for example, API).

| English Term | French Term | Canonical English | Description |
|--------------|-------------|-------------------|-------------|
| Ratio | Ratio | Ratio/Multiplier | Multiplier used for price calculation. **Important:** In pricing context, always use **Ratio** instead of **Multiplicateur**. |
| Token | Jeton | Token | API access credentials or text units processed by models. |
| Channel | Canal | Channel | Access path to API providers. |
| Group | Groupe | Group | Classification used for users or tokens. |
| Quota | Quota | Quota | Available service quota for a user. |

## Model Related

| English Term | French Term | Canonical English | Description |
|--------------|-------------|-------------------|-------------|
| Prompt | Invite | Prompt | Input content for the model. |
| Completion | Complétion | Completion | Model output content. **Important:** Do not use alternatives such as *achèvement* or *finalisation* in this context. |
| Input | Entrée | Input/Prompt | Data sent to the model. |
| Output | Sortie | Output/Completion | Data returned by the model. |
| Model Ratio | Ratio du modèle | Model Ratio | Pricing ratio for different models. |
| Completion Ratio | Ratio de complétion | Completion Ratio | Extra billing ratio for model output. |
| Price per call | Prix fixe | Price per call | Price charged per request. |
| Pay-as-you-go | Paiement à l'utilisation | Pay-as-you-go | Usage-based pricing model. |
| Pay-per-call | Paiement par appel | Pay-per-call | Flat charge per request. |

## User Management

| English Term | French Term | Canonical English | Description |
|--------------|-------------|-------------------|-------------|
| Root User | Super-administrateur | Root User | Administrator with highest privileges. |
| Admin User | Administrateur | Admin User | System administrator. |
| Normal User | Utilisateur normal | Normal User | User with standard privileges. |

## Recharge & Redemption

| English Term | French Term | Canonical English | Description |
|--------------|-------------|-------------------|-------------|
| Top up | Recharge | Top Up | Add quota to an account. |
| Redemption Code | Code d'échange | Redemption Code | Code that can be redeemed for quota. |

## Channel Management

| English Term | French Term | Canonical English | Description |
|--------------|-------------|-------------------|-------------|
| Channel | Canal | Channel | API provider access channel. |
| API Key | Clé API | API Key | API access key. **Important:** Prefer **Clé API** over generic translations of token. |
| Priority | Priorité | Priority | Channel selection priority. |
| Weight | Poids | Weight | Load-balancing weight. |
| Proxy | Proxy | Proxy | Proxy server address. |
| Model Mapping | Redirection de modèle | Model Mapping | Replace model name in request body. |
| Provider | Fournisseur | Provider/Vendor | API/AI service provider. |

## Security Related

| English Term | French Term | Canonical English | Description |
|--------------|-------------|-------------------|-------------|
| Two-Factor Authentication | Authentification à deux facteurs | Two-Factor Authentication | Extra account security check. |
| 2FA | 2FA | Two-Factor Authentication | Abbreviation for two-factor authentication. |

## Translation Guidance

### Context-Aware Prompt/Input Distinction

- **Invite / Prompt**: User interaction and LLM prompt contexts.
- **Entrée / Input**: Billing, technical documentation, and data-processing descriptions.
- **Rule**: Use *Invite/Prompt* for UI/interaction wording and *Entrée/Input* for backend or calculation contexts.

### Token Meaning by Context

- API token for access control (API Token)
- Text token processed by the model (Text Token)
- System access token for authentication flow (Access Token)

### Quota Notes

- Service quota available to a user.
- Sometimes also translated as **Crédit** depending on context.

### French Language Notes

- Keep plural forms valid for localization frameworks (`_one`, `_other`).
- Pay attention to gender and grammar in French terms.
- Examples: `modèle` is masculine, `canal` is masculine.

### Standardized Terms

- **Complétion**: model output.
- **Ratio**: multiplier for pricing.
- **Code de redemption / code d'échange**: prefer **Code d'échange**.
- **Fournisseur**: organization or service exposing API or models.

---

**Contributor note:** If you spot inconsistent terminology or have better French localization suggestions, open an Issue or Pull Request.

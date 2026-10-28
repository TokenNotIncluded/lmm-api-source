# Translation Glossary

This glossary provides reference translations for key project terminology to keep documentation and localization consistent.

## Core Concepts

| English Term | Meaning | Notes |
|--------------|---------|-------|
| Ratio | Multiplier used in price calculation | Use in all pricing logic and billing references. |
| Token | API credential or model input text unit | If possible, prefer context-specific labels such as **API token**, **text token**, or **access token**. |
| Channel | API provider entry point | Used for provider integration and traffic routing. |
| Group | User or token classification | Affects quota and pricing policy. |
| Quota | Available service quota | Numeric service entitlement for a user account. |

## Model Related

| English Term | Meaning | Notes |
|--------------|---------|-------|
| Prompt | Input content for a model | In user-facing UI, this is usually prompt-style text. |
| Completion | Model output | Output returned by the model. |
| Input | Data sent to a model | Includes prompt-like and request-context payloads. |
| Output | Data returned by a model | Includes completion and structured output payloads. |
| Model Ratio | Pricing ratio for each model | Used to scale usage costs by model family. |
| Completion Ratio | Price multiplier applied to output | Separate coefficient from input ratio. |
| Price per call | Cost per request | Flat charge model. |
| Pay-as-you-go | Usage-based billing | Charged according to consumed units. |
| Pay-per-view | Charge per invocation | Alternate naming used in legacy UI text. |

## User Management

| English Term | Meaning | Notes |
|--------------|---------|-------|
| Root User | Highest-privilege account | Usually operator-level admin role. |
| Admin User | System administrator | Can manage users, policies, and platform settings. |
| Normal User | Standard account | No elevated privileges. |

## Recharge & Redemption

| English Term | Meaning | Notes |
|--------------|---------|-------|
| Top Up | Add quota to account | Increases available balance/credits. |
| Redemption Code | Code redeemable for quota | Also used for coupon-like promotions. |

## Channel Management

| English Term | Meaning | Notes |
|--------------|---------|-------|
| Channel | Provider integration channel | Defines where API requests are routed. |
| Key | API access key | Prefer **API Key** in UI text when used for authentication. |
| Priority | Channel selection priority | Higher values determine preferred routing. |
| Weight | Load balance weight | Used for distribution across channels. |
| Proxy | Proxy server address | May include protocol and host validation rules. |
| Model Mapping | Request model remapping | Rewrites model name in request body. |

## Security Related

| English Term | Meaning | Notes |
|--------------|---------|-------|
| Two-Factor Authentication | Additional account verification step | Usually shown as 2FA. |
| 2FA | Two-Factor Authentication | Use uppercase acronym consistently. |

## Billing Related

| English Term | Meaning | Notes |
|--------------|---------|-------|
| Ratio | Price multiplier | Same as Multiplier in many contexts. |
| Multiplier | Price multiplier | Use carefully with ratio context to avoid redundancy. |

## Translation Notes

- **Prompt**: model input.
- **Completion**: model output.
- **Ratio**: multiplier used for pricing calculations.
- **Quota**: available service quota; sometimes also translated as **Credit**.
- **Token** context can mean:
  - API Token
  - Text Token
  - Access Token

---

**Contribution note:** If you find terminology issues or have better translation suggestions, please open an Issue or Pull Request.

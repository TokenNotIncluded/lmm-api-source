# LMM API Privacy Policy

Effective date: July 31, 2026

## 1. Who is responsible for your information

The organization responsible for processing personal information is LMM API, maintained by the TokenNotIncluded team.

Contact email: [support@lmm.best](mailto:support@lmm.best).

## 2. Information we process

To provide the service, we may process the following information:

- **Account details.** Your username, email address and any sign-in identifiers you choose to link.
- **Authentication and security information.** Password hashes, API keys, sign-in times, IP addresses, and device and browser information. API keys are protected by access controls and masked by default in management screens.
- **Usage and billing records.** The model used, request time, token or request usage, balance changes, orders and error records.
- **Request content.** Prompts, files you submit and generated outputs. We process this content only as necessary to forward a request, return its result, troubleshoot a problem, conduct a security audit or meet a legal obligation.
- **Support communications.** Information you choose to provide in correspondence with our support team.

Do not submit personal information you have no right to process, state secrets or other material the law prohibits you from processing.

## 3. How and why we use information

We use automated systems and, where necessary, human support to create and protect accounts, authenticate access, forward model requests, calculate charges, provide support, investigate faults, prevent fraud and abuse, improve reliability and meet legal obligations.

We do not sell your personal information or use your request content to train models intended for the public.

## 4. How long we keep information

We keep information only as long as needed for the purposes above. We generally keep account details while the account exists. We keep usage, billing and security records for the periods needed for troubleshooting, dispute handling, security audits and legal obligations. We keep support records until we resolve the issue and a reasonable period for related disputes has ended.

After account closure, we delete or anonymize relevant information within a reasonable time. We may retain information that the law requires us to keep or that we need to handle a dispute or security incident.

## 5. Processing by service providers, sharing and cross-border transfers

To fulfill the model requests you choose, we transmit the necessary request content and technical information to the relevant model or infrastructure provider. When you make a payment, we may give payment providers the information needed for the order. Some providers may be located outside your region. We make only the transfers necessary to provide the service you select, and we require the parties involved to protect the information in accordance with applicable rules. Specific models and billing terms are available on the [pricing page](/pricing).

## 6. Cookies and local storage

The website may use cookies or browser local storage to keep you signed in, remember interface preferences, maintain security settings and improve your experience. Disabling necessary storage may prevent sign-in or stop some features from working.

## 7. Your rights

Subject to applicable law, you may ask to access, copy, correct, supplement or delete your personal information. You may also withdraw consent for processing based on consent, request account closure or ask us to explain our processing rules.

Send your request from your registered email address to [support@lmm.best](mailto:support@lmm.best). We may verify your identity to protect your account. We will respond within the period required by applicable law.

## 8. Security

We use access controls, separation of privileges, protections for data in transit, audit logging and system hardening to reduce the risk of unauthorized access, disclosure, alteration or loss.

If a security incident may affect your rights or interests, we will take remedial action and meet the notification or reporting requirements of applicable law.

## 9. Minors

The service is primarily intended for people with full legal capacity to act on their own behalf. Minors should use it with a guardian’s guidance and consent. They should not submit sensitive personal information or make payments on their own.

## 10. Policy changes and contact

We may update this policy when the service or legal requirements change. We publish the new effective date on the website.

For a privacy question, complaint or rights request, contact [support@lmm.best](mailto:support@lmm.best).

## Supplement on content safety review

The built-in AI assistant and API safety audit each have an optional OpenAI Moderation feature. Both are off by default. Administrators can enable them separately and specify the account groups covered. The features stay off for groups that are not configured.

When review is enabled, we minimize the text extracted for that request, redact sensitive details as necessary and send it to OpenAI’s official Moderation API for background classification. For a recognized API protocol, we take only the text of the last user message. For the built-in assistant, we take the current user input and handle the assistant’s reply separately. We exclude the full request JSON, complete conversation history, system prompts, tool definitions and tool results. We skip unrecognized request structures. Even after these steps, the text may contain personal or sensitive information that you submitted. Do not submit personal information or secrets that you have no right to process.

Review covers text only. It excludes images, audio, video and binary files. Long text within the supported limit is reviewed in chunks. We act on a classification only when all chunks have been reviewed successfully. If content exceeds the limit or is not actually reviewed, we do not treat it as having been fully reviewed.

Chats handed over to a human support agent use the existing private support process. They are excluded from OpenAI content review and from this feature’s records of AI inputs and outputs.

We use an internal persistent task queue to run reviews. We keep the necessary classification, task status, notification, balance change and risk statistics records to administer safety, apply published rules and handle requests for review. The retention, access control, international transfer and individual rights provisions of this policy apply to these records.

We process user inputs and model outputs separately. Only user inputs that complete review and are flagged count toward a user’s risk score. Model output does not increase it. The account score takes the higher of the review score under the platform’s administrative thresholds and the existing wallet risk score. The reviewed-request count is simply a count. The score does not measure the probability of unlawful conduct or confidence in the classification. Any applicable deduction is rounded down to the smallest ledger unit and cannot exceed either the configured amount or the available positive balance. An amount below one ledger unit triggers only a warning. No new debt is created.

OpenAI’s handling of submitted information is subject to its applicable terms and [official data controls](https://developers.openai.com/api/docs/guides/your-data). This supplement does not promise that the platform or third parties will retain no information in every security or legal situation.

See the [safety review description](/legal/safety-review.html) for the feature’s scope and the [published safety rules](/security) for current group settings and category amounts. To request human review of a review record, risk statistic or balance action, use in-service support or email [support@lmm.best](mailto:support@lmm.best). Do not send passwords or complete API keys.

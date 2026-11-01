# Files

- [Browser identity and account lifecycle](browser-identity-and-lifecycle.md) - Explains Core browser registration, login and refresh sessions, MFA and account security, and the guarded account closure workflow.
- [Channel management and accounting](channel-management-and-accounting.md) - Documents Core managed channel publication, encrypted provider credentials, and display-only accounting continuity.
- [Core identity, persistence, and schema lifecycle](core-identity-and-data.md) - Explains the Rust Core's ownership of identity and authoritative financial state, and how PostgreSQL installation, runtime authority checks, and schema upgrades are separated.
- [Extensions host and service integrations](extensions-and-integrations.md) - Describes how the Go Extensions process composes selected business modules, calls the Rust Core over private Unix-socket RPC, and owns separate extension persistence.
- [Model routing, provider calls, and billing](model-routing-and-billing.md) - Traces model requests through host-owned authentication and routing, durable charge reservations, upstream execution, stream checkpoints, settlement, and recovery.
- [Payments, provider evidence, and refunds](payments-and-refunds.md) - Traces payment preparation, provider proof, Core ledger transitions, and evidence-bound refund handling across Rust Core and Go Extensions.
- [System boundaries and request flow](system-boundaries.md) - Maps the three executable programs and static Web app, their authoritative responsibilities, and the contracts callers use to cross service boundaries.

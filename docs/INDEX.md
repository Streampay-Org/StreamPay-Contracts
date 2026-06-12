# Docs Index

A single page listing every doc under `docs/` with a one-line summary.
Pair this with the table at the bottom of `README.md` if you want
external visitors to find the right entry point.

## Contributor onboarding

| Doc | Summary |
|---|---|
| [`local-development.md`](local-development.md) | First-build instructions and editor setup. |
| [`scripts.md`](scripts.md) | Helper scripts under `scripts/`. |
| [`testing-strategy.md`](testing-strategy.md) | How unit and snapshot tests are organized. |
| [`cli-cheatsheet.md`](cli-cheatsheet.md) | Common `stellar-cli` invocations against the contract. |

## Contract reference

| Doc | Summary |
|---|---|
| [`architecture-overview.md`](architecture-overview.md) | Crate layout and data flow. |
| [`glossary.md`](glossary.md) | Definitions for streaming terms. |
| [`error-codes.md`](error-codes.md) | Canonical panic-string list. |
| [`events.md`](events.md) | Event topics and payload shape. |
| [`version-encoding.md`](version-encoding.md) | `u32` packing and bump rules. |
| [`storage-layout.md`](storage-layout.md) | Persistent and instance storage layout. |
| [`ttl-strategy.md`](ttl-strategy.md) | When and why TTLs are bumped. |

## Feature specs

| Doc | Summary |
|---|---|
| [`accrual-spec.md`](accrual-spec.md) | Continuous accrual math and rounding. |
| [`cancellation.md`](cancellation.md) | Stream cancellation behavior. |
| [`collateral.md`](collateral.md) | Collateralization model. |
| [`disputes.md`](disputes.md) | Dispute resolution flow. |
| [`end-time.md`](end-time.md) | End-time semantics and edge cases. |
| [`factory-pattern.md`](factory-pattern.md) | Factory pattern roadmap. |
| [`pause-resume.md`](pause-resume.md) | Pause and resume mechanics. |
| [`rate-update-policy.md`](rate-update-policy.md) | Policy for updating `rate_per_second`. |
| [`resource-limits.md`](resource-limits.md) | Soroban resource limit considerations. |
| [`schema-versioning.md`](schema-versioning.md) | StreamInfo schema versioning. |
| [`security-token-callbacks.md`](security-token-callbacks.md) | Token callback safety notes. |
| [`stream-lifecycle.md`](stream-lifecycle.md) | Full lifecycle state machine. |
| [`timestamp-accrual.md`](timestamp-accrual.md) | Ledger timestamp assumptions. |
| [`upgradeability.md`](upgradeability.md) | Upgrade policy and constraints. |

## Operations

| Doc | Summary |
|---|---|
| [`audit-readiness.md`](audit-readiness.md) | Pre-audit checklist. |
| [`maintenance.md`](maintenance.md) | Routine maintenance playbook. |
| [`faq.md`](faq.md) | Frequently asked questions. |
| [`RELEASE.md`](RELEASE.md) | Release process. |

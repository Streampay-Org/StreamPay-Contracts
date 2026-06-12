# Security Policy

## Reporting a vulnerability

Please **do not** file public GitHub issues for security vulnerabilities.

Instead, use GitHub's private vulnerability reporting for this repository
(Security tab -> "Report a vulnerability"), if available. If private
reporting is not enabled, contact the project maintainers directly with a
minimal reproduction, impact assessment, and suggested fix.

We aim to acknowledge new reports within **3 business days** and to
provide a triage decision (accepted / needs more info / not a vulnerability)
within **10 business days**.

## Supported versions

Security fixes are applied to the latest `main` branch. Tagged releases
older than the most recent minor version are best-effort only.

| Version | Supported |
|---|---|
| `main` | Yes |
| Latest tag | Yes |
| Older tags | Best-effort |

## Scope

In scope:

- Loss of funds, freezing of funds, or incorrect accrual computed on-chain.
- Authorization bypass on `create_stream`, `start_stream`, `stop_stream`,
  `withdraw_stream`, or `archive_stream`.
- Storage corruption or TTL strategies that allow griefing.

Out of scope:

- Issues that require compromising a payer's secret key.
- Off-chain UX bugs in third-party front-ends.
- Denial-of-service that costs the attacker more than the victim.

## Safe harbor

We will not pursue legal action against researchers who:

- Make a good-faith effort to avoid privacy violations or data destruction.
- Only interact with accounts they own or have explicit permission to test.
- Report findings through the channels above rather than public disclosure.

# MuriArc documentation

> English | [简体中文](README_cn.md)

The candidate source identity is `1.0.0 / E0001 / permanent-upgrade`, but the physical RC has not passed and `v1.0.0` has not been released. Documentation must preserve that distinction.

## Product and engineering

| Document | Purpose |
| --- | --- |
| [Configuration](CONFIGURATION.md) | Runtime choice, downloads, Desktop/Server setup, variables, operation, backup, and troubleshooting |
| [Architecture](ARCHITECTURE.md) | Runtime topology, layers, domain boundaries, transactions, and adapters |
| [Security](SECURITY.md) | Trust boundaries, accounts, secrets, AI controls, audit, and reporting |
| [Environments](ENVIRONMENTS.md) | Supported toolchains, isolated worktrees, and local/CI verification |
| [MuriArc data migration](MIGRATION.md) | MuriArc schema/data-root upgrades, recovery, and import/snapshot boundaries |

## Delivery and operations

| Document | Purpose |
| --- | --- |
| [Server deployment](DEPLOYMENT.md) | Source/preview Server deployment and day-two operations |
| [Desktop delivery](DESKTOP_DELIVERY.md) | Windows Tauri runtime, data-root handling, signed updater, and acceptance |
| [Server delivery](SERVER_DELIVERY.md) | Signed Native/systemd and Managed Compose contracts for 1.0+ |
| [Upgrade Engine](UPGRADE_ENGINE.md) | `muriarcctl`, fixed state machine, locks, Journal, and trust chain |
| [Upgrade compatibility](UPGRADE_COMPATIBILITY.md) | Epoch identity, write leases, migration immutability, and rollback boundary |
| [Cloudflare Public Profile](CLOUDFLARE_PUBLIC_PROFILE.md) | Optional public ingress controls and residual risk |
| [Delivery acceptance](DELIVERY_ACCEPTANCE.md) | Automated and manual acceptance scope without claiming an RC pass |

## Architecture decisions

Architecture Decision Records are internal Chinese engineering records and use the `_cn.md` suffix. Start with [ADR-0001](adr/0001-application-layer_cn.md); the current sequence continues through ADR-0008.

## Engineering archives

`DATA_MANIFEST.md`, `log/PROJECT_LOG.md`, and `RELEASE_EVIDENCE_cn.md` are maintainer archives or internal evidence descriptions. They are intentionally excluded from the ordinary user navigation above and must not be interpreted as a public release certificate.

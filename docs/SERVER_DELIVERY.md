# MuriArc Server delivery

> English | [简体中文](SERVER_DELIVERY_cn.md)

## Status

This document defines the formal Server delivery boundary for the `1.0.0 / E0001 / permanent-upgrade` candidate and later. Contract tests, candidate source identity, and templates do not mean the physical RC has passed.

The root development Compose file remains a source/preview tool. A formal install uses exactly one signed Server bundle profile: `native-system` or `managed-compose`.

## Privilege and recovery boundary

The long-running `muriarc-server` is a low-privilege application process:

- no systemd control, Docker socket, release signing, backup orchestration, or database DDL;
- Native reads the immutable release/config and writes only its current generation data;
- Managed Compose mounts application data, never the Docker socket;
- a one-shot upgrade executor applies DDL only to an isolated Candidate;
- host `muriarcctl` owns install, backup, restore, activation, and service control.

A recoverable generation includes PostgreSQL, data/attachments, configuration, Keyset/Master Key, AI state, and `deployment-generation.json`. Restoring only a database or volume is incomplete.

## Signed bundle

The bundle packager accepts already-built final binaries and UI assets. It rejects symlinks, empty assets, traversal, existing output, and output inside Git. It emits a closed `bundle-manifest.json`; the publishing pipeline pins its object digest in signed target metadata.

The external Release Manifest describes Native, Compose, Desktop, provenance, and signature evidence after their final digests exist. It is not embedded into the bundle it describes, avoiding digest self-reference. Installation verifies both the signed outer target and closed inner bundle manifest.

## Native/systemd profile

Fixed layout:

```text
/opt/muriarc/releases/<version>/       immutable release
/opt/muriarc/current                   atomic release symlink
/etc/muriarc/server.env                root:muriarc 0640
/var/lib/muriarc/control/active.env    root:root 0600
/var/lib/muriarc/generations/<uuid>/   generation data/attachments/keyset
/var/lib/muriarc/backups/              verified recovery points
/var/lib/muriarc/candidates/           Candidate control data
```

Installation verifies and stages the bundle, installs systemd/sysusers/tmpfiles definitions, and enables—but does not automatically start—the service. The administrator creates real protected config/control files, prepares a matching generation, runs `muriarcctl doctor`, then starts the service.

`/livez` means the process is alive. `/readyz` additionally requires exact Epoch/Digest/Generation, valid activation/lease state, actual data/attachment roots, usable AI Master Key, and UI assets.

## Managed Compose profile

Managed Compose uses an absolute install root, digest-pinned images, host-owned `server.env` and `active.env`, and fixed controller invocation. It forbids `build:`, floating tags, Watchtower, direct uncontrolled `pull/up`, and Docker-socket mounts.

PostgreSQL has no host-published port; Server binds loopback. Each generation and Candidate uses non-overlapping PostgreSQL/database and data paths. Candidate disables external Providers, background jobs, and real-user writes.

## Physical Driver configuration

`muriarcctl` normally loads `bin/muriarc-physical-driver` from the immutable, receipt-verified installed bundle. `MURIARCCTL_PHYSICAL_DRIVER` is an absolute, non-symlink control-file override for controlled testing; formal production delivery leaves it unset.

The host that runs `muriarcctl` must provide PostgreSQL 17+ clients at `/usr/lib/postgresql/17/bin/pg_dump` and `/usr/lib/postgresql/17/bin/pg_restore`, and:

| Variable | Requirement |
|---|---|
| `MURIARCCTL_BACKUP_RECIPIENT_FILE` | Absolute path to a non-empty age recipients file used for encryption. |
| `MURIARCCTL_BACKUP_IDENTITY_FILE` | Absolute path to the private age identity, a non-symlink file with owner-only permissions (`0600`). |
| `MURIARCCTL_AGE_EXECUTABLE` | Optional absolute age executable; defaults to `/usr/bin/age`. |
| `MURIARCCTL_PG_DUMP_EXECUTABLE` | Optional absolute PostgreSQL 17+ `pg_dump`; defaults to `/usr/lib/postgresql/17/bin/pg_dump`. |
| `MURIARCCTL_PG_RESTORE_EXECUTABLE` | Optional absolute PostgreSQL 17+ `pg_restore`; defaults to `/usr/lib/postgresql/17/bin/pg_restore`. |
| `MURIARCCTL_POSTGRES_ADMIN_URL` | Native/systemd only: protected PostgreSQL admin URL used to create, clone, and drop isolated databases. |

Managed Compose requires the same host-side PostgreSQL clients, age executable, recipient, and identity. A running Compose service alone does not make backup/restore ready. `muriarcctl doctor` reports only readiness booleans and never prints these paths or their contents. Keep the admin URL and private identity in root-controlled configuration; never copy passwords, identities, tokens, or key material into tickets, chat, logs, or Git.

## Upgrade and maintenance window

The fixed order is signed-target verification, three locks, preflight, drain, Write Lease freeze, joint backup, actual isolated restore, Candidate migration, seven-layer verification, atomic activation, read-only startup verification, and new Write Lease.

A single node does not promise zero downtime:

- M0: UI/no schema; normally short switch;
- M1: short write freeze;
- M2: explicit read-only maintenance window;
- M3: offline structural migration.

Read-only activation is a control-plane state, not ordinary business service; even session touch may be a write. Public traffic stays gated until final readiness.

Before the new generation's first write, activation can return to the verified source. After first write, automatic downgrade is permanently refused.

## BYO and recovery points

Bring-your-own PostgreSQL/storage is accepted only if it proves PostgreSQL 17, isolated Candidate database, complete dump/restore, generation-directory copy, DDL executor, seven-layer verifier, and service control. Missing capability fails `doctor/upgrade`; it never falls back to online in-place migration.

The latest restored-and-verified recovery point is not automatically pruned. Explicit `muriarcctl recovery prune` may remove only an identified older point. Dumps, attachments, keys, Journal, and reports remain outside Git.

## Cloudflare and final RC

Public ingress uses the separate host `cloudflared` templates and [Cloudflare Public Profile](CLOUDFLARE_PUBLIC_PROFILE.md); never expose the Origin directly.

The final RC binds Native/systemd, Managed Compose, Windows Desktop, Release Manifest, artifact lock, signatures/provenance, E0001 SQLite/PostgreSQL fixtures, full history matrix, recovery/fault injection, first-write boundary, signing attacks, and Cloudflare staging to one digest set. Any FAIL, SKIP, missing physical driver, or empty Candidate Catalog blocks release.

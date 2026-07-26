# Upgrade compatibility contract

## Compatibility floor

The permanent compatibility promise starts at the first stable
`1.0 / E0001` release. The current `0.1.0` line is registered as
`preview_epoch_0` so that pre-release databases can be adopted explicitly,
tested, and converted into the first immutable fixture. A stable release may
be upgraded to the latest stable release in one administrator operation; the
controller may execute multiple internal Epoch hops.

Data is preserved only when the upgraded Application/API/real UI can read it,
continue writing old records, retrieve attachment bytes and AI history, and
retain Audit/Provenance continuity. A database file that merely still exists is
not acceptance evidence.

## Four-dimensional identity

Every runnable generation is identified by validated values in
`muriarc_core::compatibility`:

- `ApplicationVersion`;
- `DataEpoch`;
- a backend-specific `BackendStateDigest`, computed from ordered SQLx
  migration versions, descriptions, and SHA-384 checksums and then hashed with
  SHA-256;
- `GatewayContractRevision`.

`ReleaseManifest` additionally pins the SQLite and PostgreSQL identities,
PostgreSQL major version, Bootstrap/Controller protocol range, migration class,
and every deliverable by SHA-256 and size. The in-code Release Catalog and
Persistent Data Registry are append-only.

## Persistent deployment state

SQLite migration `0031` and PostgreSQL migration `0033` add:

- `muriarc_generation_sets`;
- `muriarc_upgrade_operations`;
- `muriarc_write_leases`;
- singleton `muriarc_deployment_state`.

An active generation must have an unexpired active write lease. Database-level
triggers cover every current business table, mark the generation's first write,
and reject INSERT/UPDATE/DELETE with `muriarc_write_lease_required` after the
lease is revoked. Future migrations that add business tables must install the
same fences. Control-plane tables and the SQLx ledger are excluded.

PostgreSQL migration `0034` appends credential policy revision and HMAC-keyed
login backoff state for the Cloudflare Public Profile, then re-installs the
generation write fences so the new auth table is part of the same recovery and
Write Lease boundary. It stores no probed email address or Cloudflare secret.

The data root contains `deployment-generation.json`. Server and Desktop compare
its generation, Epoch, and Backend digest with the database before opening the
application. Missing or mismatched manifests are never reconstructed during an
ordinary start.

## Startup and pre-release adoption

Long-running Server/Desktop startup calls `compatibility_report`; it does not
call the migration runner. It fails closed for a missing/changed/unknown
migration, identity drift, inactive generation, invalid lease, missing
generation manifest, or missing attachment root.

During `preview_epoch_0` only, `MURIARC_PREVIEW_BOOTSTRAP=true` is an explicit
adoption escape hatch. It applies the preview migration set, creates an initial
generation/lease, and writes the manifest. It is not a stable upgrade mechanism
and must be removed from the stable managed profile when `muriarcctl` owns
installation and upgrades. Desktop may bootstrap a provably empty fresh data
root without the flag; an existing Desktop database requires the explicit
preview flag or the signed updater.

Server refuses to create a replacement AI Master Key when encrypted credential
rows exist. It also refuses a missing/empty attachment root when attachment
metadata exists. Normal startup does not run the legacy AI profile materializer.

## Migration immutability

`migrations/checksums.json` locks every migration file. CI runs
`scripts/check_migration_checksums.py`; modifying or removing a locked file, or
adding an unregistered migration, fails. New schema work is append-only and uses
`Expand -> Backfill -> Switch -> Contract`. Uncertain changes are M3.

Persisted enum/JSON changes use `PreservedValue` and `VersionedJson`: unknown
raw values remain available and are marked `needs_review`; they are not silently
mapped to a normal business state.

## Recovery set and rollback boundary

PostgreSQL/SQLite, attachments/data, configuration, generation manifest, Keyset,
and AI state form one recovery set. A backup is valid only after an isolated
restore and verifier run. Before the new generation's first write, the
controller may atomically return to the prior generation. Once
`first_write_at` is set, automatic downgrade is forbidden; recovery requires a
forward fix or an explicit restore operation that records operator confirmation
of possible data loss.

The shared Upgrade Engine and independent muriarcctl now implement the fixed
transition/evidence model, three-lock protocol, hash-chained Journal,
PostgreSQL fencing primitives, TUF-compatible metadata validation, and fixed
Bootstrap Protocol described in [UPGRADE_ENGINE.md](UPGRADE_ENGINE.md).
Immutable release fixtures and Native/Compose delivery drivers build on these ports. The Desktop
driver now implements the shared `UpgradeDriver` contract and runs through `UpgradeEngine`, using
the same Host lock, persistent SQLite operation state and hash-chained engine Journal, Tauri
signature verification, isolated recovery/Candidate copies, attachment/AI/Audit inventory,
transactional continue-write proof, atomic locator activation and the first-write rollback boundary.
It also preserves the exact old executable with a pinned size and SHA-256 before installer launch.
A target-startup failure before first write restores the source locator and delegates to that verified
old executable; the failed installed version must never open an incompatible source database.
Cloudflare profile and final 1.0 physical RC remain separate release layers and must not duplicate or
weaken these checks.

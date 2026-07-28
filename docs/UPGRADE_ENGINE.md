# Upgrade Engine and muriarcctl

> English | [简体中文](UPGRADE_ENGINE_cn.md)

The control-plane contracts and delivery drivers are implemented, and the candidate source identity is `1.0.0 / E0001 / permanent-upgrade`. A real upgrade and release are successful only when the physical drivers and unchanged final signed artifacts produce complete evidence; that RC has not passed.

## Authority boundary

muriarc-server remains the low-privilege, long-running application. It has no
systemd, Docker socket, release-signing, backup-orchestration, or raw DDL
authority. muriarcctl is a separate local administrator process. Native,
Managed Compose, and Desktop delivery code implements UpgradeDriver; it does
not copy or reorder the shared state machine in muriarc-upgrade.

Commands that need a physical deployment Driver fail when that Driver is absent; they never print simulated success. Native, Managed Compose, and Desktop implementations connect the shared state machine to real service, backup, Candidate, and activation operations.

## Fixed state machine

The only successful path is:

    Initialized
      -> LocksAcquired
      -> PreflightPassed
      -> Drained
      -> WritesFrozen
      -> BackupCreated
      -> BackupRestored
      -> CandidatePrepared
      -> CandidateMigrated
      -> CandidateVerified
      -> Switched
      -> ReadOnlyActivated
      -> ActivationVerified
      -> WriteLeaseOpened
      -> Completed

Each transition accepts a typed evidence object. In particular:

- Preflight pins the signed target, maintenance class, free-space requirement,
  and recovery prerequisites.
- Drain must report zero requests, Jobs, attachment writers, and Provider
  requests.
- Freeze records the revoked lease and fencing token.
- Backup must contain database, attachments, data artifacts, configuration,
  Keyset, AI state, and generation manifest.
- BackupRestored requires an actual isolated restore whose digest matches the
  backup. The verified recovery point is registered before Candidate work.
- Candidate evidence requires a private endpoint with Provider, background
  work, and real-user writes disabled.
- Candidate verification requires all seven layers: asset restore, Storage,
  Store/Application, real API, real Remote UI, continued writes, and read-only
  no-side-effects.
- The generation switch must be atomic. The target first starts without a
  Write Lease and behind a traffic gate; readiness and no-write checks pass
  before the Engine creates a new fenced Write Lease.

There is no API to jump to a later phase. A Driver call may be repeated after a
crash and therefore must be idempotent for the operation/revision.

## Three locks and durable recovery

1. upgrade.lock is created exclusively at the host control-state root.
2. PostgreSQL uses a dedicated session holding
   pg_try_advisory_lock(0x4d55524955504752).
3. muriarc_upgrade_operations permits only one running operation and the active
   business Write Lease is moved through active, draining, and revoked with a
   monotonically increasing fencing token.

The database operation JSON is authoritative. A mode-0600 append-only JSONL
Journal mirrors every persisted revision with a SHA-256 hash chain. Resume
rejects a local Journal that is ahead of or conflicts with database state, but
can repair a lagging local Journal from the persistent snapshot.

recovery-points.json never permits pruning the most recently restored and
verified recovery point. Artifact deletion is Driver-owned and only occurs
after explicit muriarcctl recovery prune.

Before a target write, a failed operation may restore the source generation and
issue a higher fencing token. After the Candidate generation has
first_write_at, the Engine records recovery_required and refuses automatic
downgrade. An explicit restore must carry the operator's data-loss
confirmation.

## TUF-compatible trust and bootstrap

The trust client implements the TUF Root/Timestamp/Snapshot/Targets chain with
Ed25519 threshold signatures, canonical signed JSON, expiry checks, monotonic
metadata versions, parent length/SHA-256 pins, sequential dual-signed Root
rotation, and signed Release Manifest validation.

VerifiedRelease is non-exhaustive and cannot be deserialized, so callers
outside the crate cannot manufacture a trusted target. The fixed Bootstrap
Protocol rechecks controller protocol compatibility plus target length and
SHA-256 immediately before Unix exec, or the corresponding child-process
handoff on other platforms.

Private signing keys, database URLs, passwords, tokens, cookies, API keys, and
real recovery content never enter the Journal or Git.

## Physical execution prerequisites

The installed Server bundle contains the profile-aware `muriarc-physical-driver`; normal operation resolves it from the verified install receipt. Before any backup, upgrade, verification, or recovery command, `muriarcctl doctor` requires host `/usr/lib/postgresql/17/bin/pg_dump` and `/usr/lib/postgresql/17/bin/pg_restore`, an age executable (`MURIARCCTL_AGE_EXECUTABLE`, default `/usr/bin/age`), `MURIARCCTL_BACKUP_RECIPIENT_FILE`, and the owner-only `MURIARCCTL_BACKUP_IDENTITY_FILE`. Native/systemd also requires the protected `MURIARCCTL_POSTGRES_ADMIN_URL`. Compose service activity is not a substitute for these host capabilities.

These variables carry paths or protected connection material. Doctor exposes only readiness, and public diagnostics suppress command output. Operators inspect and provision the files locally; private identity, password, Token, Session, CSRF, or key content must never be pasted into chat or committed.

## CLI surface

    muriarcctl install --profile native-system|managed-compose
    muriarcctl doctor|status [--output json]
    muriarcctl update check
    muriarcctl upgrade [--to <version>]
    muriarcctl backup create|verify
    muriarcctl verify --read-only
    muriarcctl recovery resume [--operation <uuid>]
    muriarcctl recovery restore [--backup <uuid>] [--confirm-data-loss]
    muriarcctl recovery prune --backup <uuid>

Raw migration, --force, and skip-verification options are rejected by the
parser.

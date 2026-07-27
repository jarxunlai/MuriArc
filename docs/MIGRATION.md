# MuriArc data migration and recovery

> English | [简体中文](MIGRATION_cn.md)

## Scope

This document covers migration of MuriArc's own schema, runtime data root, configuration references, and release generations. It does not promise import from an unrelated third-party database, and the former third-party legacy-database import CLI was removed before 1.0.

Ordinary CSV/XLSX import, business Snapshot, database backup/restore, data-root relocation, and a signed version upgrade are distinct operations. None may be presented as another.

## Sources of truth

- SQL migration files are ordered, append-only schema sources for SQLite and PostgreSQL.
- `migrations/checksums.json` locks every registered migration; modifying/removing a locked file or adding an unregistered file fails CI.
- SQLx migration ledgers plus their descriptions/checksums produce the backend-specific state digest.
- Release Manifest, deployment state, Write Lease, and `deployment-generation.json` bind the application version, Data Epoch, backend state, Gateway contract, and active filesystem generation.
- The SQLite and PostgreSQL adapters must satisfy the same Store contract tests even though their migration numbers may differ for platform-specific tables.

## Startup versus upgrade

Ordinary Server/Desktop startup checks compatibility and opens only an already valid generation. It does not silently run stable schema migrations.

During `preview_epoch_0`, explicit preview bootstrap may adopt a pre-release database and create the initial generation/lease/manifest. This escape hatch is not a stable upgrade mechanism. The permanent compatibility floor begins with the final `1.0.0 / E0001` release, after E0001 fixtures have been produced by the final artifacts and accepted by the full RC matrix.

Stable install/upgrade belongs to `muriarcctl` and the shared Upgrade Engine: verify target, freeze/drain, joint backup, actual isolated restore, Candidate migration, seven-layer verification, atomic activation, and Write Lease.

## Schema change policy

New schema work follows `Expand → Backfill → Switch → Contract`:

1. **Expand** adds compatible columns/tables/indexes and Write Lease fences.
2. **Backfill** is bounded, idempotent, observable, and restartable.
3. **Switch** changes the Application read/write path only after both representations are valid.
4. **Contract** removes old structures only in a later release after the compatibility floor permits it.

Uncertain or offline structural work is migration class M3. Released SQL is never edited or rolled back. A fix is a new forward migration.

Persisted enum/JSON values use preserved/versioned wrappers where needed. Unknown raw values remain readable and `needs_review`; they are not silently mapped to a normal scientific state.

## Application-data evolution

MuriArc migrations add or evolve explicit domain relationships, including genetics definitions/records, breeding facts, observations and value history, participation genotype snapshots, attachment links, AI model profiles, visual candidates, account/session governance, technical-log retention, genotyping batches, compatibility identity, operation state, and Write Lease fences.

A schema migration does not invent scientific facts. It must not infer breeding pairs, parents, observation meaning, genotype definitions, evidence batches, or approval from ambiguous historical text/attachments. Such transformations require a separate owner-reviewed, provenance-preserving plan.

Server-only credential migrations preserve Argon2id hashes and existing account identity. AI profile migrations preserve owner, endpoint/protocol, model IDs, parameters, secret-version references, defaults, and conversation bindings. Invalid defaults may be cleared by a forward repair, but profiles, versions, secrets, and history are not deleted to make migration succeed.

## Desktop data-root relocation

Desktop relocation moves the MuriArc data root, not the OS keyring. It is scheduled through a native selection token and executes before SQLite opens:

1. integrity/FK checks and WAL checkpoint;
2. copy to staging on an allowed fixed local disk;
3. file-tree size/SHA-256 verification;
4. target SQLite integrity verification;
5. atomic locator switch.

Failure leaves the source active. The source is retained; no empty replacement database is created. SQLite, attachments, data artifacts, non-sensitive AI configuration, and generation manifest move together.

## Backup, Candidate, and rollback

A Server/Desktop recovery set joins the database, attachments, data artifacts, configuration, generation manifest, Keyset/Master Key references, and AI state. A backup is accepted only after an actual isolated restore and verification.

The Candidate uses independent storage and disables real-user traffic, external Providers, and background work. Verification covers:

1. restored assets and hashes;
2. database integrity/migration state;
3. Store/Application invariants;
4. real API reads;
5. real UI reads;
6. continued writes in a controlled transaction;
7. read-only no-side-effect behavior.

Before the target's first write, the controller may atomically restore the verified source generation. After first write, automatic downgrade is forbidden; use a forward fix or explicit recovery with operator confirmation of possible data loss.

## Import/export and Snapshot boundary

Ordinary import currently supports explicit Animal Registry and experiment Measurement workflows; evidence-backed genotyping batches use their dedicated workflow. Ordinary export produces a scoped Animal Registry product. Import/export is not general entity sync, database restore, or Desktop-to-Server migration.

Business Snapshot is typed JSONL plus attachments and checksums for integrity/offline retention. It excludes account/session/token secrets and is not a runnable database backup. General restore/apply remains unavailable until full preflight, cross-entity transactions, apply ledger, canonical hashes, Lab mapping, and Audit/Provenance semantics are frozen and tested.

## Acceptance

Migration acceptance must cover fresh schema, idempotent replay, every supported incremental state, interrupted resume, realistic restored copies, attachment bytes, AI history/secret references, Audit/Provenance, business invariants, and first-write rollback boundaries on both SQLite and PostgreSQL 17.

A skipped database suite, hand-edited SQL, cleared data, or “the file still exists” is not a pass.

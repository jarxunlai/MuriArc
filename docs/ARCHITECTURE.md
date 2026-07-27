# Architecture

> English | [简体中文](ARCHITECTURE_cn.md)

## Scope

MuriArc is one product with two runtime editions. Desktop and Server share the Vue interface, Application services, domain model, Store ports, import/snapshot services, AI safety layer, and compatibility contracts. Runtime-specific code supplies transport, authentication, secret storage, persistence, and delivery adapters.

```text
Vue UI ── LocalTauriGateway ── Tauri commands ──┐
                                                ├─ Application ── Core/Domain ── Store ports
Vue UI ── RemoteHttpGateway ── Axum /api/v1 ────┘                               ├─ SQLite
AI workspace ── approved domain tools ──────────────────────────────────────────└─ PostgreSQL
External client ── REST or MCP + scoped token ────────────────────────────────────────┘
```

## Layering

### UI and transport

- Vue views render state, collect user intent, and display validation/approval results. They do not own business invariants.
- `LocalTauriGateway` maps the same UI contracts to Tauri commands without exposing a local HTTP server.
- `RemoteHttpGateway` uses the Axum API, HttpOnly sessions, CSRF, and bounded JSON or streaming endpoints.
- REST/MCP handlers authenticate, authorize, deserialize, and call Application services. They do not construct multi-step domain mutations independently.

### Application

Application services normalize inputs and orchestrate use cases across domain and Store ports. A public behavior should have one Application path shared by Desktop and Server whenever the runtime boundary permits it.

Typical responsibilities include animal registration and transfer, project assignment, breeding transitions, experiment publication/enrollment, measurement drafts, import confirmation, snapshot creation, AI conversation workflows, and human approval.

### Core and domain

Core has no dependency on Tauri, Axum, SQLx database implementations, or model Providers. It contains:

- typed identifiers, revisions, actors, sources, Audit, and Provenance;
- animals, cages, lifecycle events, and project-animal assignments;
- genetics definitions, records, evidence-backed batches, pedigrees, breeding lines, pairs, mating events, litters, and drafts;
- experiment templates, cohorts, participation, procedures, observations, measurements, samples, and attachments;
- permissions, AI operation contracts, compatibility identity, and release-manifest types.

Domain invariants reject invalid transitions before persistence. Examples include terminal animals that cannot be revived, revision-checked mutations, one-time signing, immutable published templates, explicit project scope, valid breeding member composition, and append-only observation history.

### Store ports and adapters

SQLite and PostgreSQL adapters implement the same Store contracts and must pass shared contract tests. Adapters own SQL, transaction boundaries, database-specific constraints, and migration primitives; the Core does not.

All formal writes require an actor, source, revision, Audit, and—where applicable—Provenance. Core records are soft-deleted unless a documented technical-retention policy applies.

## Transaction boundaries

One scientific intent is one transaction. For example:

- animal registration writes the Animal, lifecycle event, Audit, and Provenance together;
- transfer locks and validates source/target cages, advances the Animal revision, writes the event, and records Audit atomically;
- registering an offspring draft creates the Animal, both pedigree relations, lifecycle history, Audit, and Provenance in one commit;
- experiment enrollment stores the participation and the genotype evidence snapshot used at that time;
- approved measurement or genotyping-batch writes preserve human approval, evidence links, and provenance.

Transport code may never emulate these transactions with a sequence of unrelated Store calls.

## Data and asset boundaries

Database rows store attachment metadata and content hashes; large bytes live in the edition-specific attachment store. Reads verify object identity and size/hash where the operation requires integrity. Private AI source images and extraction candidates stay outside ordinary project visibility until a human approves a formal relation.

A business Snapshot is a typed, checksummed archive for integrity and offline retention. It is not a runnable database backup and currently has no general restore/apply operation. Deployment recovery uses a coordinated database, attachments, data artifacts, configuration, generation manifest, key material, and AI-state recovery set.

## AI architecture

Each user owns versioned model profiles. A conversation binds an immutable profile version; defaults are explicit references rather than implicit “first model” fallback. Provider construction resolves the protocol, normalized endpoint, model identifier, capability declaration, parameters, and user-scoped secret for that exact version.

The model sees only fixed, typed domain tools that survive the intersection of:

1. authenticated human permissions;
2. lab/project scope;
3. external-token scopes, if present;
4. declared conversation autonomy;
5. the executor's currently available capabilities.

Raw SQL and security/transport proofs are rejected before Provider access. Reads return bounded projections and citations. Writes return a reviewable draft; sensitive or research-signing operations require explicit human steps.

Vision follows either the bound conversation model or an explicitly selected vision relay. Uploaded images are size/type/dimension checked, sanitized, privately stored, and converted to candidates. A model never chooses authoritative Animal/Experiment identifiers for final writes.

## Identity and tenancy

The Server hierarchy is Environment → Lab → Project. `EnvironmentRoot` is deployment-owned recovery/governance authority; it does not silently replace lab/project authorization. Users receive lab roles and optional project roles. Project-scoped reads must not leak other animals in the same cage or unrelated experiments.

Desktop uses a local operator profile rather than Server credential tables. Its passwordless entry only confirms the active operator inside the trusted OS account. It must not be described as data encryption or an access-control boundary.

## Runtime and delivery boundaries

- **Desktop**: Tauri v2, bundled Vue assets, SQLite, a local data root, and OS keyring references. Formal delivery is a Windows WebView installer, not VNC/noVNC.
- **Server**: Axum, PostgreSQL, responsive Web UI, and loopback-first ingress. Long-running `muriarc-server` has no Docker socket, systemd control, release-signing key, backup orchestration, or raw DDL authority.
- **Upgrade control plane**: `muriarcctl` and the shared Upgrade Engine own signed-target verification, freeze/drain, backup/restore, Candidate verification, atomic activation, and Write Lease transitions.

The candidate source identity is `1.0.0 / E0001 / permanent-upgrade`. The permanent compatibility and delivery promise becomes active only after the unchanged final artifact set passes the complete physical RC gates.

## Architecture decisions

Internal Chinese ADRs record the detailed choices: [ADR-0001](adr/0001-application-layer_cn.md), [ADR-0002](adr/0002-workspace-tenancy_cn.md), [ADR-0003](adr/0003-transaction-boundaries_cn.md), [ADR-0004](adr/0004-genetics-v2-compatibility_cn.md), [ADR-0005](adr/0005-breeding-atomicity_cn.md), [ADR-0006](adr/0006-observation-version-policy_cn.md), [ADR-0007](adr/0007-enrollment-genotype-snapshot_cn.md), and [ADR-0008](adr/0008-runtime-identity-and-account-security_cn.md).

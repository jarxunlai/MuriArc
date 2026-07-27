# Delivery acceptance

> English | [简体中文](DELIVERY_ACCEPTANCE_cn.md)

## Status and evidence rule

This checklist describes implemented product scope and the evidence required for development/manual acceptance. MuriArc remains `0.1.0 / preview_epoch_0`; no result in this document should be read as a formal `1.0.0 / E0001` RC pass.

Automated checks, a dirty-main development service, an unsigned Tester package, or a source-built Compose stack are useful development evidence but not final artifact acceptance.

## Delivered scope

- Animals, cages, lifecycle events, transfer, project assignment, attachments, Audit, and Provenance.
- Structured genetics definitions/records and evidence-backed genotyping batches.
- Breeding lines, colonies, one-male/multi-female pairs, retirement, mating events, litters, animal drafts, and pedigrees.
- Versioned experiment templates, experiments, cohorts, participation, enrollment genotype snapshot, procedures, typed observations, measurements, and samples.
- Bounded animal/measurement import, scoped Animal Registry export, and checksummed business Snapshot.
- Desktop SQLite/Tauri and Server PostgreSQL/Axum sharing Application/Core/Store contracts and Vue behavior.
- Persistent Server accounts, roles, Environment Root governance, sessions/CSRF, revocable tokens, and technical-log retention.
- User-isolated, versioned multi-Provider AI profiles, conversations, citations, controlled tools, autonomy limits, vision routing, private images, and human-approved candidates.
- Compatibility identity, generation manifest, Write Lease, Upgrade Engine, signed delivery contracts, and fail-closed RC definitions.

## Automated gates

A change affecting delivery should run the applicable gates on the exact clean commit:

### Rust and databases

- migration checksum immutability and locked Cargo metadata;
- formatting and zero-warning Clippy;
- Core/Application/AI/data/import/snapshot tests;
- SQLite and real PostgreSQL 17 Store contracts;
- Server account/API/MCP/AI tests;
- Upgrade, delivery, release-evidence, and verifier tests;
- fresh, idempotent, incremental, interrupted/resume, and no-leftover-database checks.

A PostgreSQL suite skipped for missing configuration is not a pass.

### UI and Desktop

- branding consistency;
- dependency high-severity audit;
- Vue unit tests and typecheck;
- remote and local production builds;
- responsive Desktop/Tablet/Mobile Playwright;
- Windows Desktop missing-updater-key negative test, strict Clippy/tests, and Tauri smoke build.

### Containers and documentation

- Compose configuration, image build, health, persistent login, and clean teardown without deleting persistent acceptance data;
- bilingual filename/status contract and local Markdown links;
- scan for obsolete origin claims and sensitive/generated files.

## Manual acceptance

Use synthetic data and a disposable account/environment. Do not paste access credentials into the report.

### A. Runtime editions

1. Desktop opens a native Tauri window, uses SQLite, and remains functional without Server.
2. Server serves the responsive UI through Axum/PostgreSQL with login, CSRF, logout, and scoped roles.
3. Desktop and Server expose equivalent supported business behavior while preserving their different identity/security models.

### B. Accounts and isolation

1. Reconcile the configured Environment Root and verify old sessions are revoked after password rotation.
2. Verify Root/LabAdmin/ProjectAdmin/AnimalManager/Editor/Viewer boundaries.
3. Verify suspension, soft deletion, forced password change, external-token expiry/revocation, and project isolation.
4. Confirm no response/log/Audit exposes passwords, hashes, cookies, CSRF, Tokens, keys, or object paths.

### C. Animals, genetics, and breeding

1. Register animals and cages; transfer with revision/capacity checks and an auditable lifecycle event.
2. Assign an animal to a project and confirm unrelated project users cannot see cage mates.
3. Create multi-component genotype definitions and records; confirm old/unknown values remain explicit.
4. Create a breeding pair, mating event, litter, and offspring draft; register the draft atomically with both parents and provenance.
5. Create/confirm/reject/void an evidence-backed genotyping batch and verify attachment relations.

### D. Experiments and records

1. Publish a versioned template and prevent mutation of the published version.
2. Enroll an animal and verify the genotype evidence snapshot does not change when later genotyping records arrive.
3. Create procedures, typed observations, measurement drafts/signatures, samples, and attachments.
4. Verify immutable/mutable/versioned observation policies and historical value retrieval.

### E. Data operations

1. Preview animal/measurement import with field mapping, ambiguity, units, duplicates, and project scope.
2. Confirm a valid import atomically and verify rejected/conflicting input leaves no partial state.
3. Export a scoped Animal Registry and verify spreadsheet-formula neutralization.
4. Create/verify a business Snapshot and confirm private AI operations/account secrets are excluded.
5. Confirm Snapshot is not offered as general restore and import/export is not labeled migration.

### F. AI

1. Create separate profiles for at least two users and prove profile, model, parameter, and secret isolation.
2. Validate OpenAI Chat Completions, Responses, and Anthropic mappings only against mock upstreams.
3. Verify no-key, archived model, stale default, legacy read-only conversation, timeout, output limit, and sanitized Provider-error states.
4. Verify Ask/Auto/Full cannot exceed human permission or sensitive-operation exclusions.
5. Verify data reads include bounded citations; writes create reviewable drafts rather than direct scientific facts.
6. Upload/sanitize private images, test direct vision and explicit relay, then reject or human-approve a candidate.

### G. Desktop delivery

1. Build from the exact clean GitHub commit in a disposable Windows account/VM.
2. Verify local data-root relocation, restart, integrity, and fail-closed missing-disk behavior.
3. Verify OS-keyring isolation and no key bytes in SQLite/backup/report.
4. Exercise old-data update, interruption/resume, pre-first-write fallback to the verified old executable, and post-first-write downgrade refusal.

### H. Server delivery and public profile

1. Verify Native/systemd and Managed Compose separately from final signed packages.
2. Perform joint backup and actual isolated restore; run all seven Candidate layers.
3. Verify drain/freeze, atomic activation, read-only gate, new Write Lease, and rollback boundary.
4. On Cloudflare staging, verify direct-Origin rejection, HTTPS/cookies/CSRF, login backoff, WAF/rate limits/cache, 95 MiB edge boundary, and external API dual control.

## Known limitations

- The current repository has not completed the physical `1.0.0 / E0001` RC.
- Business Snapshot has no general restore/apply.
- Ordinary import/export is deliberately narrow and is not database migration or synchronization.
- macOS formal packaging/acceptance is not complete.
- Cloudflare no-MFA Public Profile retains account-takeover risk and must not be described as MFA.
- AI acceptance excludes real Provider keys from CI; project owners perform optional real-Provider testing privately.

## Formal 1.0 RC

The formal RC uses final signed Native/systemd, Managed Compose, and Windows Desktop artifacts plus one `artifact-lock.json` and Release Manifest. The same digests must produce E0001 SQLite/PostgreSQL fixtures and pass the full history, recovery, fault-injection, first-write, signature-attack, and Cloudflare staging matrix with `FAIL=0` and `SKIP=0`.

Only that unchanged artifact set may be published as `v1.0.0`. Tester prereleases and dirty-main development services never satisfy this gate.

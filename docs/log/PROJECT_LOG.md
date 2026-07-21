# Project log

## 2026-07 | Independent MuriArc repository baseline

MuriArc is maintained as an independent project from this baseline. Legacy source repositories and
owner-managed databases are migration inputs only; they are not deployment targets and are not
managed from this repository.

Delivered baseline:

- Rust workspace with shared domain and application layers, SQLite/PostgreSQL Store adapters,
  Axum Server, Tauri Desktop, migration, import/export, attachment and snapshot boundaries.
- Animal lifecycle, cage, breeding, genetics, experiment, measurement, sample, observation,
  provenance and audit workflows.
- Passwordless local Desktop entry and authenticated shared Server operation with hierarchical
  account governance and credential lifecycle controls.
- Responsive Vue UI, Docker deployment assets, synthetic fixtures, contract tests and end-to-end
  acceptance coverage.
- Central MuriArc branding, Apache-2.0 licensing and required upstream attribution.

Privacy and data boundary:

- Legacy database paths, fingerprints, record counts, conflict statistics and acceptance reports
  remain in an owner-managed private archive outside Git.
- Databases, attachments, snapshots, secrets, runtime configuration and real animal records must
  never be committed or included in a Docker build context.
- Migration conflicts remain reports until the project owner explicitly reviews them; this log does
  not imply that anomalous legacy data has been manually confirmed.

Known release gates:

- Snapshot restore/apply remains intentionally unavailable until full typed preflight,
  cross-entity/attachment transactions, an application ledger and canonical conflict semantics are
  implemented.
- Multi-replica Server deployment requires shared rate limiting in addition to process-local guards
  and reverse-proxy controls.
- Public release requires a dedicated personal-information, secret, fixture and artifact audit of
  the complete reachable Git history and release assets.

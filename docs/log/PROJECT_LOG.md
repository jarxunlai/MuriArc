# Project log

## 2026-07 | Independent MuriArc release baseline

MuriArc is independently developed and maintained by `jarxunlai`, with AI-assisted engineering implementation. The project is released under Apache-2.0. AI is not a legal author or copyright holder.

Delivered baseline:

- Rust workspace with shared Domain/Application layers, SQLite/PostgreSQL Store adapters, Axum Server, Tauri Desktop, import/export, attachments, Snapshot, AI, and upgrade-control boundaries.
- Animal lifecycle, cages, project assignment, breeding, genetics, experiments, observations, measurements, samples, attachments, Audit, and Provenance workflows.
- Passwordless local operator confirmation for Desktop and authenticated multi-user Server operation with hierarchical governance and credential lifecycle controls.
- Responsive Vue UI, source/preview Compose assets, synthetic fixtures, contract tests, responsive end-to-end coverage, and signed-delivery contracts.
- Central MuriArc branding, `jarxunlai` notice, and Apache-2.0 licensing.

Privacy and data boundary:

- Runtime databases, attachments, snapshots, recovery copies, secrets, environment configuration, and real animal/research records never enter Git or a Docker build context.
- Standard acceptance fixtures contain synthetic data only.
- Migration/upgrade anomalies remain fail-closed reports until the project owner explicitly reviews the data; this archive never records assumed human approval.

Known release gates:

- The candidate source identity is `1.0.0 / E0001 / permanent-upgrade`; the physical RC has not passed.
- Snapshot restore/apply remains unavailable until typed preflight, cross-entity/attachment transactions, apply ledger, and canonical conflict semantics are implemented and accepted.
- Multi-replica Server requires shared rate limiting in addition to process-local and reverse-proxy controls.
- Public release requires a complete reachable-history and artifact audit for personal information, secrets, fixtures, generated files, and signing provenance.

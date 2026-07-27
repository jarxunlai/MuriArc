# Data manifest

## MuriArc logo master

- source: Project owner-provided image
- local_path: `branding/logo-master.png`
- managed_by: Git (lightweight brand input)
- used_by: UI, Tauri icon generation, Web favicon/PWA assets
- status: stable
- notes: Preserve the supplied mark; derived sizes must retain its internal white background.

## Runtime databases and attachments

- source: User-generated animal and experiment records
- local_path: OS application-data directory or Server volumes
- managed_by: MuriArc runtime and backup/snapshot jobs
- used_by: Desktop or Server instance
- status: active
- notes: Excluded from Git; database, attachment volume, secrets, and audit data must be backed up together. Domain snapshots include explicit project-animal assignments, but exclude AI operation state, Server technical access logs, and the technical-log retention policy; use a full database backup when those records must be restored. Formal Audit and Provenance are never subject to technical-log cleanup.

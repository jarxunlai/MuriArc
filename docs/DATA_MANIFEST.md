# Data manifest

## Legacy MurisPro runtime database

- source: Existing local MurisPro installation
- local_path: Owner-managed external legacy database (not stored in Git)
- managed_by: Project owner
- used_by: `muriarc-legacy-migrator` audit and migration acceptance
- status: legacy
- notes: Never commit or modify in place. The owner-managed private acceptance manifest records the source fingerprint; migration verifies it before and after every acceptance run.

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
- notes: Excluded from Git; database, attachment volume, secrets, and audit data must be backed up together.

# MuriArc Desktop delivery

> English | [简体中文](DESKTOP_DELIVERY_cn.md)

## Status and runtime shape

The formal Desktop target is a Windows Tauri v2 WebView installer containing the Vue UI and using `LocalTauriGateway` over Tauri IPC. It does not expose a local HTTP API and is not delivered through VNC/noVNC, a browser remote desktop, or Server Docker.

The candidate source identity is `1.0.0 / E0001 / permanent-upgrade`, but the physical RC has not passed. A local debug build or unsigned Tester package is not a signed release.

- SQLite: `<data-root>/muriarc.sqlite3`
- Attachments: `<data-root>/attachments/`
- Data artifacts: `<data-root>/data/`
- Generation identity: `<data-root>/deployment-generation.json`
- Provider keys: OS keyring only
- Local entry: operator confirmation inside the trusted Windows account, not a password/security lock

## Data root and relocation

The OS application-data **config root** retains the locator and migration intent. The selected **data root** keeps SQLite, attachments, data artifacts, non-sensitive AI configuration, and generation identity together.

A user may select an empty directory on a fixed local disk. The installer directory, relative paths, UNC/network shares, symlinks, the current root, and parent/child overlaps are rejected. The native picker returns a one-time selection token; Vue cannot submit an arbitrary filesystem path.

Relocation is scheduled, then performed before any SQLite pool, attachment service, or Provider setting opens on the next start:

1. checkpoint WAL and run integrity checks;
2. copy to an isolated staging directory;
3. compare a SHA-256 file-tree manifest;
4. open and verify the target database;
5. atomically update the locator.

Failure leaves the source active and never creates a replacement empty database. The source is retained for explicit recovery. OS keyring entries and WebView2 cache are not moved.

## Signed updater and Candidate activation

Formal updates use HTTPS updater metadata, Tauri/Minisign signatures, an independently signed Release Manifest, and pinned artifact size/SHA-256. Release builds fail if the configured updater public key is missing or invalid.

Before installer handoff, Desktop preserves the exact old executable under an operation-scoped recovery directory and records its size/SHA-256. The target resumes through the shared Upgrade Engine before opening business storage:

1. reverify target and Release Manifest;
2. acquire host/backend locks and reconcile persistent operation state with the hash-chained Journal;
3. checkpoint/verify source and create a complete recovery copy;
4. actually restore that copy into an isolated Candidate;
5. migrate only the Candidate;
6. verify integrity/FKs, Store/Application reads, attachment bytes, AI history/secret references, Audit inventory, transactional continue-write, and no-side-effect read-only behavior;
7. atomically switch the locator without a Write Lease;
8. verify target startup/readiness, then open the new Write Lease.

Before the target's first business write, failure may atomically return the locator to the source and delegate to the verified old executable. After `first_write_at`, automatic downgrade is forbidden; only a forward fix or explicit recovery with data-loss confirmation is allowed.

Provider key bytes are not copied. Same-machine update keeps the OS-keyring account; cross-machine recovery restores profile/history references but requires the user to enter the key again.

## Exact-commit Windows build

Every distributed package must come from an exact, clean GitHub commit—not an old clone, a moving `origin/main`, or a previous local build. The build record must include:

- 40-character commit SHA and clean-tree proof;
- Rust/Node/pnpm/Tauri versions;
- updater public-key identity without private material;
- installer/bundle name, size, SHA-256, and signature/provenance evidence;
- external build/evidence directories outside Git.

Private signing keys and passwords exist only in the protected release environment. They must not enter Git, build transcripts, Release Manifest custom metadata, or acceptance attachments.

A release build runs the Windows CI equivalents: local-gateway UI build, missing-updater-key negative test, strict Desktop Clippy, Desktop tests, and Tauri no-bundle/build packaging gates.

## Runtime acceptance

Use a disposable Windows account or VM with no real MuriArc data or personal AI keys. Verify:

- native window and bundled UI, without a source server;
- local operator confirmation and offline operation;
- fresh SQLite/data-root creation and safe relocation/restart;
- keyring isolation among Windows users and MuriArc profile versions;
- animal, cage, breeding, experiment, observation, measurement, sample, attachment, Audit, and AI profile workflows;
- old-data upgrade, interrupted update resume, verified old-executable fallback before first write, and refusal to downgrade after first write;
- uninstall/reinstall behavior without silently deleting user data or recovery points.

macOS must not be published until equivalent real-device packaging, keychain, update, migration, and recovery acceptance exists.

## Windows Tester package

A friend-testing package is separate from the formal RC/release:

- build from an already merged, clean, traceable GitHub commit;
- include only synthetic standard data;
- scan the archive for API Key, password, Session, Token, CSRF, private key, and real research data;
- publish under a tester-specific prerelease tag;
- label it **unsigned**, **synthetic data**, and **not for production**;
- provide the package SHA-256.

Tester evidence and artifacts must never be mixed with the final `v1.0.0` artifact lock or RC report.

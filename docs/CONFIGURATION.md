# MuriArc configuration guide

> English | [简体中文](CONFIGURATION_cn.md)

## Scope and release status

This is the public entry point for choosing, downloading, configuring, operating, and backing up MuriArc Desktop or Server. The source identity is `1.0.0 / E0001 / permanent-upgrade`, but the physical 1.0 RC has not passed and no official `v1.0.0` release is claimed.

The current Windows and Server Tester artifacts are unsigned test deliveries. They are not production releases and do not constitute formal RC evidence. Use only synthetic data until you have approved a real-data policy and recovery procedure.

## 1. Choose a runtime

| Need | Recommended runtime | Storage | Network | Delivery boundary |
| --- | --- | --- | --- | --- |
| One researcher on one trusted Windows account | Windows Desktop Tester | Local SQLite, attachments, and OS keyring | No Server required | Unsigned Tester; synthetic test data only |
| Try the shared UI on one computer | Server Docker Tester on Docker Desktop/Linux | Private PostgreSQL and `server_data` named volumes | Loopback `127.0.0.1` | Unsigned; empty by default, optional synthetic standard-v1 |
| Multiple users on a trusted LAN or VPN | Server Docker Tester behind a private HTTPS reverse proxy | Same two named volumes plus host-owned `.env` | Private LAN/VPN only | Evaluation only; not an Internet or production profile |
| Develop or inspect source | Clean Git checkout/worktree | Worktree-isolated runtime data | Developer controlled | Source build; not a published artifact |
| Production, public ingress, signed upgrades, or real-data recovery | Formal signed Server delivery | Joint database/data/config/key recovery set | Approved private or Cloudflare profile | Not yet published; follow [Server delivery](SERVER_DELIVERY.md) |

Desktop is Tauri + SQLite. Server is Axum + PostgreSQL. Docker Desktop on Windows x64 runs the `linux/amd64` Server Tester image; no native Windows Server image is provided.

## 2. Download and verify

Use the [GitHub Releases page](https://github.com/jarxunlai/MuriArc/releases).

- Windows Tester: choose the prerelease whose tag begins `tester-v1.0.0-standard-v1-`.
- Server Docker Tester: choose the prerelease whose tag begins `server-tester-v1.0.0-standard-v1-`.
- Never substitute a similarly named third-party archive or a floating container tag.

Verify the downloaded file before extracting.

Linux/macOS/WSL:

```bash
sha256sum --check MuriArc-*.zip.sha256
```

PowerShell:

```powershell
$Expected = (Get-Content .\MuriArc-*.zip.sha256).Split(' ')[0]
$Actual = (Get-FileHash .\MuriArc-*.zip -Algorithm SHA256).Hash.ToLowerInvariant()
if ($Actual -ne $Expected) { throw 'SHA-256 mismatch' }
```

The Server Tester ZIP also contains `CHECKSUMS.sha256`. After choosing and editing an environment template, run `muriarc-tester.sh verify` or `muriarc-tester.ps1 verify`. Verification checks every bundled file, Compose policy, and the two immutable image references.

## 3. Windows Desktop first start

1. Verify and extract the Windows Tester ZIP to a user-owned directory.
2. Run the included verification launcher. Do not bypass a checksum or manifest error.
3. The first run copies the immutable standard-v1 synthetic baseline to the current Windows user's LocalAppData. Later edits stay in that user data root and do not rewrite the extracted baseline.
4. Desktop uses SQLite and a local attachment/data root. It is not a browser Server and must not be shared by placing its database on a network drive.
5. Back up the complete Desktop data root while MuriArc is closed. Keep the SQLite database, attachments, generated artifacts, storage marker, and key references together.
6. Each Windows user adds their own AI Provider profile and API Key in Settings. The Key is referenced through the OS keyring; it does not belong in a project file, screenshot, or shared configuration.

The unsigned Tester is for evaluation. A formal Desktop delivery additionally requires a signed installer/updater, a clean GitHub commit, and accepted release evidence. See [Desktop delivery](DESKTOP_DELIVERY.md).

## 4. Server Docker prerequisites

Install:

- Docker Engine or Docker Desktop with Linux containers;
- Docker Compose v2 (`docker compose version`);
- an amd64 CPU/runtime, at least 4 GiB available memory, and sufficient durable disk;
- `curl` and `sha256sum` for Bash, or PowerShell 7/Windows PowerShell for the `.ps1` script.

The Docker user controls the database and can inspect process configuration. Restrict Docker membership. Do not run this Tester on an untrusted shared host.

Extract the Server Tester ZIP, then choose one template:

```bash
cp .env.empty.example .env  # recommended: brand-new empty database
# or
cp .env.demo.example .env   # explicit synthetic standard-v1 demo
chmod 600 .env
```

Replace every `REPLACE_` value. Use different Compose project names and host ports for multiple copies. Generate URL-safe secrets so the PostgreSQL URL remains valid:

```bash
openssl rand -hex 32  # PostgreSQL password
openssl rand -hex 32  # use a different value for the Environment Root
```

Do not paste `.env`, resolved `docker compose config`, `docker inspect`, cookies, CSRF values, Tokens, Provider requests, or logs containing private object paths into an issue.

## 5. Initialize empty or demo data

### Empty database (default)

```bash
./muriarc-tester.sh verify
./muriarc-tester.sh init-empty
```

PowerShell:

```powershell
.\muriarc-tester.ps1 verify
.\muriarc-tester.ps1 init-empty
```

`init-empty` refuses any existing project container or named volume. Only after proving both volumes absent does it temporarily enable preview bootstrap. It waits for `/readyz`, stops Server, and restarts the same deployment with bootstrap disabled. Do not manually leave bootstrap enabled.

### Synthetic standard-v1 demo

```bash
./muriarc-tester.sh verify
./muriarc-tester.sh init-demo
```

Demo initialization also requires brand-new volumes. It starts private PostgreSQL, runs the bundled strict PostgreSQL Seeder, verifies the fixed dataset digest and domain counts, installs the matching generation manifest, then starts Server with bootstrap disabled.

The demo Lab and User UUIDs must remain:

- Lab: `4d555249-4152-4300-0000-000000000001`
- Environment Root/User: `4d555249-4152-4300-0000-000000000002`

You provide the Root email, display name, and password. Startup reconciles the existing synthetic User into the Environment Root and creates LabAdmin membership and credentials. A second initialization is rejected. If seeding or verification fails, preserve the site; do not clear it or patch SQL.

## 6. Server Tester environment variables

| Variable | Required/default | Sensitivity | Change impact |
| --- | --- | --- | --- |
| `MURIARC_TESTER_DATASET_MODE` | Required: `empty` or `demo` | Public | Must match the one-time init command; never switch an existing deployment |
| `MURIARC_COMPOSE_PROJECT_NAME` | Default in template | Public | Names containers/networks/volumes; changing it selects a different deployment |
| `MURIARC_TESTER_SOURCE_COMMIT` | Fixed by the bundle | Public | Must match the image/fixture commit; do not edit |
| `MURIARC_TESTER_SERVER_PORT` | `8787` | Public | Changes only the loopback host port |
| `MURIARC_POSTGRES_DB` | `muriarc` | Internal | Changing after initialization points Server at another database |
| `MURIARC_POSTGRES_USER` | `muriarc` | Internal | Must match the PostgreSQL volume owner/configuration |
| `MURIARC_POSTGRES_PASSWORD` | Required, 32+ URL-safe characters | Secret | Needed by DB and Server; changing requires coordinated PostgreSQL credential rotation |
| `MURIARC_DATA_ROOT` | Template-fixed path | Internal | Part of the recovery/generation boundary; do not change after init |
| `MURIARC_ATTACHMENT_ROOT` | Template-fixed path | Internal | Must be backed up with PostgreSQL; changing can look like data loss |
| `MURIARC_AI_MASTER_KEY_FILE` | Template-fixed path | Critical secret path | The generated file decrypts per-user Provider credentials; loss blocks AI credential recovery |
| `MURIARC_LAB_ID` | Stable UUID; demo value fixed | Internal identity | Changing may conflict with existing records and fails closed |
| `MURIARC_LAB_NAME` | Required display name | Public inside the lab | Reconciled on Server start |
| `MURIARC_ROOT_USER_ID` | Stable UUID; distinct from Lab; demo value fixed | Internal identity | Changing selects/reconciles another Root and can revoke sessions |
| `MURIARC_ROOT_USER_EMAIL` | Required | Personal/sensitive | Login identifier; must be unique after normalization |
| `MURIARC_ROOT_USER_NAME` | Required | Personal/sensitive | Displayed in UI and Audit actor information |
| `MURIARC_ROOT_PASSWORD` | Required, 32+ unique URL-safe characters | Critical secret | Reconciliation rotates the credential and revokes affected sessions |
| `MURIARC_SESSION_COOKIE_SECURE` | `false` for direct loopback HTTP | Security control | Must be `true` behind HTTPS; a Secure cookie is not sent over plain HTTP |
| `MURIARC_SESSION_TTL_HOURS` | `12`, valid `1..720` | Security control | Changes future session expiry policy |
| `RUST_LOG` | Server/info default | Operational | More verbose logs increase disclosure risk; never enable body/secret logging |

Image references are rendered directly into `compose.yaml` as immutable `@sha256:` values. The bundle creates no `latest` tag. PostgreSQL has no host port, and Server publishes only `127.0.0.1:<port>`.

Do not add `MURIARC_AI_MASTER_KEY`, bootstrap bearer Tokens, Provider API Keys, Cloudflare secrets, or a Docker socket mount to this Tester `.env` or Compose file.

## 7. Environment Root, sessions, and private reverse proxy

The Environment Root is the recovery administrator. On every read-write Server start, MuriArc transactionally reconciles the configured Lab, Root User, LabAdmin membership, and Argon2id credential. Identity collisions, cross-Lab ownership, deleted records, invalid email, or unsupported credentials fail closed.

Browser login returns an HttpOnly session cookie and a CSRF token. Never copy either value. Logout, password rotation, account disablement, membership revocation, expiry, and forced-password-change state are enforced against persistent state.

Direct local use:

```text
http://127.0.0.1:8787
MURIARC_SESSION_COOKIE_SECURE=false
```

For a trusted LAN or VPN, keep the container port on loopback and place a host-owned reverse proxy in front of it. Terminate HTTPS, allow only the chosen private hostname, disable caching for authenticated APIs, set `MURIARC_SESSION_COOKIE_SECURE=true`, and restrict the host firewall/VPN. This unsigned Tester is not supported on the public Internet. Public Cloudflare deployment belongs to the formal [Cloudflare Public Profile](CLOUDFLARE_PUBLIC_PROFILE.md), not this bundle.

## 8. Per-user AI configuration

MuriArc does not ship a shared Provider key. Each authenticated Server user opens AI Settings and creates a user-owned Provider/model profile with:

- protocol and exact Base URL;
- model ID and capability flags;
- context, input, output, and history budgets;
- timeout/temperature controls;
- that user's own API Key.

The key is encrypted with the deployment AI Master Key and scoped to the user/profile version. A validation timeout or budget error leaves the form unsaved and does not imply the normal chat context window is the same value. Do not put a Provider Key in `.env`, a Lab preset, a screenshot, logs, or a friend's test package.

## 9. Operate, stop, and recover

```bash
./muriarc-tester.sh status
./muriarc-tester.sh logs
./muriarc-tester.sh down  # containers stop; named volumes remain
./muriarc-tester.sh up
```

The PowerShell script provides the same commands. No `destroy` operation is included. `up` refuses an uninitialized site. `down` never passes `--volumes`.

A valid Server backup is a coordinated recovery set containing:

1. PostgreSQL volume/database;
2. `server_data` including attachments, generation manifests, data artifacts, and the generated AI Master Key;
3. the exact `.env`, Compose ZIP manifest, source commit, and image digests.

Restore into an isolated project name and verify login, `/readyz`, projects/animals, attachment bytes, Audit/Provenance, AI history references, and a new write. A business Snapshot alone is not a Server disaster-recovery backup.

## 10. Troubleshooting and prohibited actions

| Symptom | Safe action |
| --- | --- |
| `verify` reports a checksum mismatch | Delete that extraction, re-download the release asset, and verify the outer SHA-256 |
| Image cannot be inspected anonymously | Confirm the GHCR package is Public and the digest matches the manifest; do not substitute a tag |
| Port already in use | Stop the other service or change `MURIARC_TESTER_SERVER_PORT` before initialization |
| `init-*` reports an existing volume | Use `up` for that deployment; use a new project name for a new evaluation |
| `/readyz` fails | Run `status` and inspect redacted `logs`; preserve volumes and record the release tag/commit |
| Demo verification reports drift | Stop and preserve the site; do not seed again, clear rows, or SQL-patch it |
| AI credentials cannot decrypt | Restore the matching `server_data`/Master Key with the same PostgreSQL backup |
| Browser login loops behind HTTPS | Confirm proxy scheme/host handling and `MURIARC_SESSION_COOKIE_SECURE=true` |

Never run `docker compose down --volumes`, delete named volumes, clear PostgreSQL, modify migration history, hand-edit standard-v1 rows, mount `/var/run/docker.sock`, expose `5432`, publish `8787` on `0.0.0.0`, or treat this unsigned package as production/RC evidence.

Developers building from source should use a clean worktree and follow [Environments](ENVIRONMENTS.md) and [Server deployment](DEPLOYMENT.md). Formal Server artifacts and upgrade control remain documented separately in [Server delivery](SERVER_DELIVERY.md) and [Upgrade Engine](UPGRADE_ENGINE.md).

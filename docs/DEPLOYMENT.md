# MuriArc Server deployment

> English | [简体中文](DEPLOYMENT_cn.md)

## Scope and status

For a user-facing choice between Windows Desktop, local Docker, and a private remote Server, begin with the [Configuration guide](CONFIGURATION.md). The Server Docker Tester described there is an unsigned evaluation artifact; this document continues with source-checkout and formal-delivery boundaries.

This guide covers source-checkout development of the shared Server edition. The candidate source identity is `1.0.0 / E0001 / permanent-upgrade`, but the root Compose file is not a signed release deliverable and the physical RC has not passed. Stable Native/systemd and Managed Compose contracts are documented in [Server delivery](SERVER_DELIVERY.md).

MuriArc Server is Axum + PostgreSQL + the responsive Vue UI. The application port is published on loopback by default. PostgreSQL must remain private, and production TLS terminates at a trusted reverse proxy or the documented Cloudflare Tunnel profile.

Desktop is a separate Tauri + SQLite edition and is not deployed through Docker, VNC, or noVNC.

## 1. Prepare configuration

```bash
cp .env.example .env
chmod 600 .env
```

Replace every placeholder. Required groups include:

- PostgreSQL database, role, and password;
- durable data and attachment roots;
- stable Lab and Environment Root UUIDs, display values, and Root password;
- cookie security and lifetime;
- AI Master Key source/version;
- source-checkout bootstrap kept false by default and enabled only for a disposable empty development stack;
- optional external API and MCP origins.

Generate independent values rather than reusing a personal password:

```bash
openssl rand -hex 32
openssl rand -base64 24
uuidgen  # Lab ID
uuidgen  # Root user ID; must differ from Lab ID
```

Mode `600` protects against ordinary users, not the host administrator, Docker daemon, process-environment collection, or an unencrypted backup. Encrypt configuration backups, restrict Docker membership, and never attach `.env`, `docker inspect`, or resolved Compose output to an issue.

### Environment Root

Server reconciles the configured Root on every start under a PostgreSQL transaction and advisory lock. It creates or verifies the Lab, User, LabAdmin membership, and Argon2id credential. Identity conflicts, soft-deleted records, duplicate normalized email, cross-Lab ownership, or unsupported hashes fail closed.

To rotate Root identity or password, edit the host-owned environment and restart Server. Successful credential change revokes old Root sessions. The UI cannot read the old password or silently rewrite the deployment file.

### AI Master Key

A genuinely empty deployment may generate one stable 32-byte Base64 key in the protected data-root secrets directory when no environment key is supplied. Back up that file with PostgreSQL, attachments, configuration, and generation metadata.

If encrypted credential rows exist and the original key is unavailable, startup fails and never generates a replacement. Keep `MURIARC_AI_MASTER_KEY_VERSION` unchanged until a documented rotation has re-encrypted every existing user/profile secret. Users provide their own Provider keys; no key means no external request.

## 2. Validate and start the source stack

```bash
docker compose config --quiet
docker compose build server
docker compose up -d --wait --wait-timeout 180
curl --noproxy '*' --fail http://127.0.0.1:8787/api/v1/health
```

The root Compose stack is intended only for development acceptance and keeps bootstrap disabled by default. Set `MURIARC_PREVIEW_BOOTSTRAP=true` only for a disposable empty local stack; never use it to relabel or repair existing data, and never bypass stable `muriarcctl` upgrade control.

Use `docker compose ps` and redacted application logs for diagnostics. Do not paste environment, cookies, CSRF, Tokens, passwords, Master Keys, Provider bodies, or private object paths into logs or tickets.

## 3. Browser session and CSRF

Login returns an opaque HttpOnly cookie plus a CSRF value for the active session. The UI holds the CSRF value in memory and sends it on state-changing requests. Production requires HTTPS and `MURIARC_SESSION_COOKIE_SECURE=true`.

Logout revokes the current session. Password change and Root environment reconciliation revoke other affected sessions. Suspended/deleted users, revoked memberships, expired tokens, and forced-password-change state are enforced on every authenticated request.

## 4. Reverse proxy and origin boundary

A conventional reverse proxy should:

- terminate TLS;
- forward only the intended application host;
- preserve request size/time limits;
- keep PostgreSQL and the container network private;
- forward WebSocket/streaming behavior only where required;
- avoid caching authenticated API or private attachment responses.

Do not trust arbitrary forwarded-host/proto headers. Configure exact trusted origins for browser MCP access. A non-browser MCP client still requires a revocable AI-scoped token.

For public exposure, use [Cloudflare Public Profile](CLOUDFLARE_PUBLIC_PROFILE.md); do not directly open port 8787 to the Internet.

## 5. External tokens and MCP

Persistent external tokens are user-bound, revocable, expiring, and scope-limited. They can only narrow the live user's permissions. External bearer REST/MCP is disabled by default in production/public profiles.

Bootstrap bearer values are preview adapters, not normal production credentials. If temporarily enabled, use distinct high-entropy values, keep them outside Git, and remove them after persistent login/token workflows are available.

## 6. Backup and restore

Back up as one recovery set:

- PostgreSQL;
- data and attachment roots;
- deployment configuration;
- `deployment-generation.json` and control state;
- AI Master Key/Keyset and non-plaintext AI state.

A backup is not accepted until restored into an isolated environment and verified through Storage, Store/Application, real API/UI reads, attachment bytes, AI history references, Audit/Provenance, and continued-write invariants. Never test restoration against the only live copy.

Ordinary business Snapshot does not replace a database/attachment recovery set.

## 7. Operations checklist

Before a non-development launch:

1. Confirm the exact source/artifact identity and current release status.
2. Verify all placeholders are replaced and secret files have restricted ownership/mode.
3. Keep PostgreSQL private and expose only loopback to the proxy/Tunnel.
4. Verify Secure cookie, exact origin, session lifetime, and external API policy.
5. Verify Root login, forced-password-change behavior, logout, CSRF, suspension, and token revocation.
6. Verify AI user isolation with mock Providers; never borrow Root's key for another user.
7. Create and actually restore a joint recovery set.
8. Record health, compatibility, storage, UI, and generation results without secrets.

For the 1.0+ signed upgrade and maintenance-window workflow, continue with [Server delivery](SERVER_DELIVERY.md) and [Upgrade Engine](UPGRADE_ENGINE.md).

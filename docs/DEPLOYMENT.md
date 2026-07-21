# MuriArc Server deployment

The shared edition is an Axum server backed by PostgreSQL and serving the same responsive Vue application used by the desktop edition. Axum listens on the container network, while the provided Compose file publishes its host port only on loopback by default; terminate TLS in Caddy, Nginx, or an equivalent reverse proxy.

This document is only for the shared Server deployment. The personal Desktop
edition is delivered as a Windows Tauri WebView installer backed by local SQLite
and OS keyring storage; it is not deployed through Docker, VNC, noVNC, or a
browser remote desktop. See [DESKTOP_DELIVERY.md](DESKTOP_DELIVERY.md) for the
Desktop local delivery boundary.

The shared edition uses persistent Argon2id credentials and revocable PostgreSQL
sessions. Browser session secrets are held only in an HttpOnly, SameSite=Strict
cookie; PostgreSQL stores SHA-256 token and CSRF digests, never their plaintext.
TLS remains mandatory because `MURIARC_SESSION_COOKIE_SECURE` defaults to `true`.

## 1. Prepare configuration

```bash
cp .env.example .env
chmod 600 .env
```

Generate independent database and Environment Root passwords plus stable Lab/Root UUIDs:

```bash
openssl rand -hex 32
openssl rand -base64 24
uuidgen # MURIARC_LAB_ID
uuidgen # MURIARC_ROOT_USER_ID; it must differ from the Lab UUID
```

Fill every required value in `.env`, including:

```dotenv
MURIARC_LAB_ID=<stable-lab-uuid>
MURIARC_LAB_NAME=<laboratory-display-name>
MURIARC_ROOT_USER_ID=<stable-root-user-uuid>
MURIARC_ROOT_USER_EMAIL=<root-login-email>
MURIARC_ROOT_USER_NAME=<root-display-name>
MURIARC_ROOT_PASSWORD=<environment-managed-password>
```

If the database password contains URL-reserved characters, URL-encode it before constructing `MURIARC_DATABASE_URL` for direct (non-Compose) execution. Keep the real `.env` outside Git and at mode `600`; verify with `stat -c '%a %n' .env`.

`MURIARC_ROOT_PASSWORD` deliberately remains plaintext in the host environment under the approved deployment model. Mode `600` does **not** protect it from the host administrator, Docker daemon or `docker inspect`, process-environment collection, or anyone who can read an unencrypted `.env` backup. Encrypt and access-control configuration backups, restrict Docker membership, and never attach Compose configuration or container-inspection output to tickets.

### Encrypted per-user AI credentials

The shared AI transport is opt-in. Generate one stable 32-byte Base64 master
key and protect `.env` and its backup with the same care as the database backup:

```bash
openssl rand -base64 32
```

Set the output as `MURIARC_AI_MASTER_KEY` and keep
`MURIARC_AI_MASTER_KEY_VERSION=1`. The key is never stored in PostgreSQL. It is
used only to encrypt each user's Provider credential with AES-256-GCM and
user/version-bound additional authenticated data. Losing or changing this key
makes existing credentials unreadable; do not increment the version until a
documented re-encryption workflow is available.

If the key is empty, invalid, or absent, the Server never falls back to
plaintext: `/api/v1/ai/settings`, `/api/v1/ai/turns`, and
`/api/v1/ai/approvals` remain disabled.

Provider exits are managed inside MuriArc by LabAdmin. The official
`https://api.openai.com/v1` endpoint is built in. Every other
OpenAI-compatible URL and every `LocalHttp` URL must be added as an exact
Provider endpoint in the AI management page before users can save it in their
personal AI settings. OpenAI-compatible custom endpoints must use HTTPS.
Provider HTTP clients reject redirects, so approving one URL cannot silently
redirect requests elsewhere. Use an HTTPS reverse proxy and network policy for
any non-loopback Provider. Never put a user's Provider API key in environment
variables; users save their own key through the authenticated settings endpoint,
which never echoes it back.

MuriArc reconciles the Environment Root on **every** Server start under one PostgreSQL transaction and advisory lock. It creates or verifies the Lab, Root User, LabAdmin membership, and Argon2id credential. A changed Root email/name is synchronized. If the environment password no longer verifies against the database hash, the hash, password-change timestamp, and credential revision are updated. Root identity or credential changes revoke all Root browser Sessions. Every write has a stable, sanitized Audit operation; plaintext passwords and hashes are never recorded.

Startup fails rather than guessing when it finds a duplicate normalized email, a configured Root User ID owned by another Lab, a soft-deleted Lab/User/Root membership, an unsupported credential hash, or another identity conflict. Root remains an application-level LabAdmin plus a deployment-only `isEnvironmentRoot` marker:

1. Environment Root can govern every application account and is the only identity that can create, modify, suspend, demote, or reset LabAdmin accounts.
2. LabAdmin can govern non-LabAdmin users and laboratory business, but cannot modify deployment configuration, code, Environment Root, or a peer LabAdmin.
3. ProjectAdmin is restricted to authorized Projects.
4. AnimalManager, Editor, and Viewer retain their Lab Registry, project-write, and read-only boundaries.

To change the Root password or configured identity, edit `.env` and restart only the Server. The application intentionally provides no Root profile/password editor, reset action, suspension, or role-demotion endpoint. After restart, verify login and Audit, then confirm old Root Sessions have been revoked. Environment-managed password rotation does not silently rewrite `.env` from the UI.

Deployments upgrading from the removed persistent bootstrap seed **must** map the intended existing administrator to `MURIARC_ROOT_USER_ID` and explicitly set all five `MURIARC_ROOT_*`/Lab name values before starting this version. There is no fallback to `MURIARC_BOOTSTRAP_PASSWORD`, and the old seed variables are ignored; this prevents silently retaining a forgotten bootstrap password.

Optional `MURIARC_BOOTSTRAP_TOKEN` and `MURIARC_BOOTSTRAP_MCP_TOKEN` remain controlled-preview bearer adapters only. They must be independent 32+ character secrets and use the live configured Root identity, so suspension/role/credential gates are still read from PostgreSQL. Leave both empty for normal operation: they are environment secrets rather than database-revocable external tokens.

## 2. Web sessions and CSRF

`POST /api/v1/auth/login` returns the CSRF token in JSON and sets the opaque
session cookie. The UI keeps the CSRF token in memory and sends it as
`X-CSRF-Token` on every cookie-authenticated method other than GET, HEAD, OPTIONS,
or TRACE. After a page reload, `GET /api/v1/auth/csrf` safely reconstructs the
same session-scoped token; it never returns the HttpOnly session secret, accepts
browser sessions only, and uses `Cache-Control: no-store`. Login, current-session,
CSRF recovery, and logout endpoints are:

```text
POST /api/v1/auth/login
GET  /api/v1/auth/session
GET  /api/v1/auth/csrf
POST /api/v1/auth/logout
POST /api/v1/auth/password/change
PATCH /api/v1/auth/profile
```

`GET /api/v1/auth/me` is a compatibility alias for the current-session endpoint.

Stable browser contract:

```http
POST /api/v1/auth/login
Content-Type: application/json

{"email":"researcher@example.org","password":"..."}
```

Success sets `muriarc_session=<opaque>; Path=/; HttpOnly; SameSite=Strict;
Secure` and returns:

```json
{
  "data": {
    "user": {
      "id": "uuid",
      "lab_id": "uuid",
      "email": "researcher@example.org",
      "display_name": "Researcher",
      "lab_roles": ["lab_admin"],
      "project_roles": [{"project_id": "uuid", "role": "viewer"}],
      "authentication": "session",
      "must_change_password": false,
      "is_environment_root": false
    },
    "csrf_token": "mac_...",
    "expires_at": "RFC3339 timestamp"
  },
  "request_id": "uuid"
}
```

`GET /api/v1/auth/session` returns the same `user` object in `data`.
`GET /api/v1/auth/csrf` returns
`{"data":{"csrf_token":"mac_...","expires_at":"RFC3339"},"request_id":"uuid"}`
for a live cookie session and rejects bearer credentials.
`POST /api/v1/auth/logout` requires the cookie plus `X-CSRF-Token` and returns
204 while expiring the cookie. Invalid login/session credentials return 401 with
`error.code="unauthorized"`; missing or incorrect cookie CSRF returns 403 with
`error.code="csrf_failed"`; an unavailable authentication backend returns 503
with `error.code="authentication_unavailable"`. Authentication responses use
`Cache-Control: no-store` and never echo passwords or session secrets in JSON.

Session duration is controlled by `MURIARC_SESSION_TTL_HOURS` (default 12,
integer range 1–720 hours). Set
`MURIARC_SESSION_COOKIE_SECURE=false` only for explicit loopback HTTP development;
never use that override on a network deployment.

### Password lifecycle and account governance

A user created through `POST /api/v1/admin/users` supplies `temporaryPassword` and starts with `mustChangePassword=true`. Login succeeds so the browser can establish a Session and CSRF token, but until `POST /api/v1/auth/password/change` succeeds every business route and external bearer capability returns HTTP 403 with `error.code="password_change_required"`. Only current Session/CSRF inspection, logout, and password change remain available. The UI hides business navigation and holds the user on `/change-password`.

Passwords require 8 or more Unicode characters, no more than 1024 UTF-8 bytes, no control characters, and a new value different from the current value. No character-class recipe or periodic expiry is imposed. Password fields are cleared after every attempt; strength labels are advisory only.

```http
POST /api/v1/auth/password/change
X-CSRF-Token: mac_...
Content-Type: application/json

{"currentPassword":"...","newPassword":"..."}
```

A successful self-change preserves the current Session and revokes every other Session. Normal users may change only their display name through `PATCH /api/v1/auth/profile`; administrators maintain a subordinate user's email/name through `PATCH /api/v1/admin/users/{id}/profile`.

`POST /api/v1/admin/users/{id}/password-reset` accepts an administrator's current password, the target `expectedCredentialRevision`, and a new `temporaryPassword`. It never returns or exposes the previous password, sets `mustChangePassword=true`, increments credential revision, and revokes all target Sessions and external tokens. Account administration requires a live cookie Session, CSRF, current-password step-up, revision checks, and the Root/LabAdmin hierarchy above. Environment Root endpoints return `environment_root_managed`; peer LabAdmin governance returns `lab_admin_managed_by_environment_root`.

## 3. Start the containers

```bash
docker compose config
docker compose build
docker compose up -d
docker compose ps
curl --fail http://127.0.0.1:8787/api/v1/health
```

PostgreSQL has no host port. The Server is published only as `127.0.0.1:8787`; firewall rules should expose the reverse proxy on 80/443, not port 8787 or PostgreSQL.

## 4. HTTPS reverse proxy

Minimal Caddy example:

```caddyfile
muriarc.example.org {
    encode zstd gzip
    reverse_proxy 127.0.0.1:8787
}
```

Minimal Nginx location:

```nginx
location / {
    proxy_pass http://127.0.0.1:8787;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
}
```

Use a valid certificate, redirect HTTP to HTTPS, keep the host firewall enabled, and restrict administrative access to the laboratory network or VPN.

## 5. External tokens and MCP boundary

Authenticated users create, list, and revoke their own external tokens through
`/api/v1/auth/tokens`. The raw token is returned exactly once; only its SHA-256
digest, scopes, expiry, and revocation metadata are persisted. Effective access is
always the intersection of the user's current Lab/Project roles and token scopes.
Suspending or soft-deleting the user immediately invalidates sessions and tokens.

```http
POST /api/v1/auth/tokens
X-CSRF-Token: mac_...
Content-Type: application/json

{"name":"analysis-agent","scopes":["read","export"],"expires_in_days":90}
```

The response contains `data.token` once plus `data.details`. Use that value as
`Authorization: Bearer mat_...`. `GET /api/v1/auth/tokens` lists metadata only;
`DELETE /api/v1/auth/tokens/{id}` revokes a token. These management routes accept
browser sessions only, and mutations require CSRF.

`POST /mcp` accepts only bearer identities explicitly narrowed with AI scopes.
Normal Web sessions are rejected. The first release exposes fixed read-only
domain tools and never accepts raw SQL.

Browser clients send an `Origin` header. MuriArc denies all browser origins unless `MURIARC_MCP_ALLOWED_ORIGINS` contains an exact comma-separated match, for example:

```dotenv
MURIARC_MCP_ALLOWED_ORIGINS=https://muriarc.example.org,https://ai-gateway.example.org
```

Do not use wildcards. Non-browser clients normally omit `Origin`, but still need
an unexpired, non-revoked external token with `read` scope and the underlying
user's permissions. The optional bootstrap MCP token is only a controlled-preview
fallback and should be empty in normal deployments.

## 6. Backup and restore

Create encrypted, access-controlled PostgreSQL backups on a separate system:

```bash
docker compose exec -T db sh -lc 'pg_dump -U "$POSTGRES_USER" -d "$POSTGRES_DB" --format=custom' > "muriarc-$(date +%F).dump"
```

Test restore regularly against a disposable database:

```bash
docker compose exec -T db sh -lc 'createdb -U "$POSTGRES_USER" muriarc_restore_test'
docker compose exec -T db sh -lc 'pg_restore -U "$POSTGRES_USER" -d muriarc_restore_test --clean --if-exists' < muriarc-YYYY-MM-DD.dump
```

Current MuriArc snapshots are integrity/export artifacts and cannot restore a deployment. Back up the attachment volume together with PostgreSQL, and verify attachment SHA-256 checksums after a tested restore.

## 7. Operations checklist

- Pin reviewed image tags and apply OS/PostgreSQL security updates.
- Keep `.env`, reverse-proxy credentials, AI provider keys, and backups out of Git.
- Keep `.env` mode 600, encrypt its backups, restrict Docker access, and confirm optional bootstrap bearer variables are empty.
- Rotate the Environment Root password only by editing `.env` and restarting the Server; verify the old Sessions are revoked.
- Revoke unused sessions/external tokens and review authentication audit growth.
- MuriArc itself serializes reinforced AI password checks per user session. Five
  failures within 15 minutes trigger a 15-minute in-memory cooldown; a successful
  verification clears that session's failure state. Monitor the structured
  `security_event=ai_step_up_password_failed` and
  `security_event=ai_step_up_verification_abandoned` warnings without logging
  submitted credentials.
- The built-in AI step-up limiter is process-local: it resets on restart and is not
  shared by multiple replicas. Multi-replica deployments need an additional shared
  limiter, and all deployments should apply conservative reverse-proxy rate limits
  to both `/api/v1/auth/login` and `/api/v1/ai/approvals/*/decision` as defence in
  depth. Alert on repeated authentication failures without logging request bodies.
- Monitor `/api/v1/health`, container restarts, disk usage, PostgreSQL logs, and audit growth.
- Confirm unknown `/api/v1/*` paths return JSON 404 rather than the Vue entry page.
- Run migrations and restore drills against copies before upgrading a real laboratory.
- Never share SQLite over a network drive and never point migration tools at the only copy of a legacy database.

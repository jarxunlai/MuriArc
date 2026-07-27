# Security

> English | [简体中文](SECURITY_cn.md)

## Security status

This document describes implemented controls and mandatory deployment boundaries for the current `0.1.0 / preview_epoch_0` line. It is not a claim that a public `1.0.0 / E0001` RC or an external security audit has passed.

## Trust boundaries

Treat browser input, uploaded files, REST/MCP clients, model output, Provider responses, reverse-proxy headers, backup media, and update metadata as untrusted until validated by the owning boundary.

Desktop's passwordless local entry is only operator confirmation within a trusted OS account. Server is the account-security edition and requires authenticated, scoped access.

## Server identity and sessions

- Passwords are stored as Argon2id PHC hashes; plaintext passwords and hashes are never returned through administration APIs.
- Session and external-token plaintext is shown only where the protocol requires it. PostgreSQL stores digests, not reusable plaintext.
- Browser sessions use HttpOnly, SameSite cookies and a session-derived CSRF value. Production uses Secure cookies behind HTTPS.
- User, credential, membership, expiration, revocation, suspension, soft deletion, and forced-password-change state are read live, so governance changes take effect immediately.
- Login backoff is HMAC-keyed and bounded. Public endpoints must not reveal whether an email exists.

`EnvironmentRoot` is declared by deployment configuration and reconciled transactionally. Its password is changed through deployment-secret rotation and restart, not a UI that can expose the existing credential. Root authority does not bypass project/lab data authorization.

## Authorization

Permissions are evaluated using actor identity, lab role, project role, resource relation, operation, and revision. External tokens can only narrow the human's effective permissions. Disabled external API mode unmounts or rejects bearer access before tool execution.

Project users see only explicitly assigned animals and related research records. Sharing a cage, lab, or attachment store is not sufficient authorization.

## AI Provider secrets

- **Server**: Provider secrets are encrypted per user/profile version with AES-256-GCM and identity-bound associated data. Master-key version rotation must decrypt and re-encrypt all existing ciphertext before activation.
- **Desktop**: API keys remain in the OS keyring; SQLite stores versioned opaque references, never the key.
- A Root, Editor, or Viewer profile cannot read or use another user's Provider configuration.
- API keys, passwords, sessions, CSRF, tokens, private signing keys, and decrypted Provider bodies are excluded from Debug output, Audit, normal logs, snapshots, and UI state.
- If encrypted rows exist, Server refuses to synthesize a replacement Master Key.

## AI execution controls

- No raw SQL, arbitrary URL fetching, arbitrary filesystem access, account/permission mutation, migration, or deployment control is exposed to the model.
- Tool schemas are fixed and bounded. Authorization and scope are checked again at execution time.
- Ask/Auto/Full autonomy never overrides human permission, project scope, researcher signing, reinforced approval, or operation-specific exclusions.
- Model writes become reviewable drafts. Animal transfer/death, deletion, bulk import, user governance, research signing, image evidence approval, and technical-log deletion remain human operations.
- Provider/model failures return stable sanitized diagnostics without echoing credentials, bodies, or internal database errors.

## Uploads, attachments, and private AI assets

File names are plain metadata, never paths or response headers. Uploads enforce independent size, extension, media, structure, pixel/frame, and decompression limits. Images are sanitized before model access. Object keys and hashes are verified before reads/removals; traversal and symbolic-link attacks fail closed.

Private AI images, sources, prompts, candidates, and jobs are owner-scoped. They do not enter project scope or business snapshots until a human creates an approved formal relation.

## Audit, Provenance, and logs

Formal business writes keep actor, source, revision, timestamp, Audit, and relevant Provenance. Audit is not a secret store: transport proofs, key material, private object paths, and sensitive Provider payloads are excluded.

Server technical access logs are separate from formal Audit/Provenance. Retention is count/day bounded and only Environment Root may preview policy changes or manually clean them. Formal Audit and Provenance are not subject to technical-log cleanup.

## Database and upgrade safety

Released migration files and checksums are immutable. Server/Desktop ordinary startup performs compatibility checks and does not silently migrate an existing stable database. Schema change, backup, isolated restore, Candidate verification, activation, and Write Lease changes belong to the upgrade control plane.

Database, attachments, data artifacts, configuration, generation manifest, key material, and AI state are one recovery set. Before first target write, a verified source generation may be restored atomically. After first write, automatic downgrade is forbidden; use a forward fix or an explicit operator-confirmed restore.

Never clear production data, hand-edit migration SQL, replace a Master Key, or delete an attachment volume to work around an upgrade failure.

## Network and deployment

- Keep PostgreSQL and internal service ports private.
- Publish the application on loopback by default and terminate TLS at a trusted reverse proxy or the documented Cloudflare Tunnel profile.
- Do not mount the Docker socket into `muriarc-server` or run it as the upgrade/backup authority.
- Enable Secure cookies and exact trusted origins for production.
- External REST/MCP access is disabled by default in the Cloudflare Public Profile and requires explicit host plus Cloudflare service-token controls when enabled.

See [Server deployment](DEPLOYMENT.md) and [Cloudflare Public Profile](CLOUDFLARE_PUBLIC_PROFILE.md).

## Reporting

Do not open a public issue containing real animal data, credentials, logs with identifiers, database/attachment archives, or update signing material. Use a private maintainer channel, provide the affected version and reproducible synthetic steps, and rotate any potentially exposed secret before sharing diagnostics.

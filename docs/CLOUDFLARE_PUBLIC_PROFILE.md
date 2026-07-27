# Cloudflare Public Profile

> English | [简体中文](CLOUDFLARE_PUBLIC_PROFILE_cn.md)

## Status and scope

This optional profile exposes one MuriArc Server origin through Cloudflare Tunnel. It is designed for the signed Native/systemd or Managed Compose delivery profiles; it is not proof that public production hosting or the `1.0.0 / E0001` RC has passed.

```text
Browser -> Cloudflare Edge -> cloudflared (host service) -> 127.0.0.1:8787 -> MuriArc
                                                           PostgreSQL remains private
```

`cloudflared` is an independent low-privilege host process. `muriarc-server` continues to bind loopback. Port 8787, PostgreSQL, Docker socket, control files, and Candidate endpoints are never publicly exposed.

## Security boundary

A browser-facing research application without enforced Cloudflare Access MFA has residual credential-phishing and account-takeover risk. This profile therefore requires compensating controls, but does not claim they are equivalent to MFA:

- long user passwords and Argon2id storage;
- HMAC-keyed login backoff with generic responses;
- Secure, HttpOnly, SameSite session cookies and CSRF;
- exact host/origin checks;
- Cloudflare managed rules, WAF/rate limiting, bot/abuse monitoring, and no authenticated caching;
- immediate session/token revocation for suspended or changed accounts;
- separate Environment Root governance and protected host configuration;
- regular recovery rehearsal and security-log review.

If the deployment owner requires MFA, use an upstream identity/access design that is separately specified and tested. Do not describe this no-MFA profile as providing MFA.

## Native/systemd

Install the profile's `cloudflared` service/template from the verified Server bundle. Store the Tunnel token/credential as root-owned host secret material, not in the MuriArc environment, database, bundle, Git, or application logs.

The service routes one exact public hostname to loopback. It must not use broad wildcard ingress, direct Origin DNS, or a route to PostgreSQL/metrics/control endpoints. Start MuriArc readiness first, then enable Tunnel traffic.

## Managed Compose

`cloudflared` remains a host service rather than a container with Docker-socket access. The managed Compose application continues to publish only `127.0.0.1:8787`. Do not add a public port mapping, `network_mode: host`, Watchtower, floating image tag, or socket mount.

## Login and password controls

Cloudflare rate limits complement—but never replace—Server-side HMAC-keyed login backoff. Rate-limit keys and responses must not reveal whether an email exists. Root credentials remain environment-owned and are not reset through public endpoints.

Monitor repeated authentication failures, token misuse, unexpected countries/ASNs, WAF events, and application security events without logging passwords, cookies, CSRF, Tokens, or Provider keys.

## WAF, limits, and caching

- Cache static fingerprinted UI assets only.
- Bypass cache for API, authentication, MCP, downloads, private images, attachments, and health responses carrying deployment state.
- Preserve streaming and timeout behavior needed by bounded AI calls.
- Apply method/path-aware rate limits to login, password change, uploads, AI turns, token creation, and downloads.
- Reject ambiguous host/proxy headers and direct Origin access.

The edge upload envelope is capped at **95 MiB** so Cloudflare limits fail predictably. Application endpoints keep their stricter independent limits (for example ordinary JSON, import streams, and AI images). The edge allowance does not raise an application limit.

## External REST/MCP

External bearer REST/MCP is off by default. If explicitly enabled, requests require the exact public host, Cloudflare service-token headers, and a live MuriArc user-bound scoped token. Cloudflare credentials do not grant MuriArc permission, and a MuriArc token does not bypass the edge policy.

Browser MCP origins are an exact allowlist. Non-browser clients normally omit `Origin` but still require the same live scoped authorization.

## Data and Provider traffic

Cloudflare terminates public TLS, but PostgreSQL, backups, key material, and Provider credentials remain private to the host/application boundaries. Provider API traffic goes from MuriArc to the user-configured endpoint and is not tunneled through the public browser route.

Private images and attachments keep their owner/project authorization; a guessed URL, cached response, or Cloudflare authenticated identity is not sufficient.

## RC gate

Cloudflare staging is a required physical scenario for the formal RC. Evidence must use the final signed bundle/image digests and include:

- exact hostname/Tunnel/Origin topology;
- direct-Origin rejection;
- HTTPS, cookie, CSRF, login-backoff, WAF/rate-limit, and cache behavior;
- 95 MiB edge boundary plus stricter application limits;
- external API off by default and dual-control behavior when enabled;
- recovery, restart, configuration rotation, and log-redaction checks;
- zero FAIL and zero SKIP.

Contract tests or a local `cloudflared` template alone are not RC PASS.

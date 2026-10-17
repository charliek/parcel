# Changelog

## v2026.10.17

- The sign-in button reads "Stride Labs" in production (`AUTH_SLAUTH_LABEL`).
- Dependency advisories patched (lock only): h2 0.4.20 (RUSTSEC-2026-0258),
  rustls 0.23.46, quinn-proto 0.11.19; chacha20 off a yanked release.

## v2026.10.16

First production release: `parcel.stridelabs.ai` (app) and
`parcelcontent.stridelabs.ai` (content) on Cloud Run + Neon + GCS. The repo
is public on GitHub; CI runs the Rust, functional, frontend and Playwright
suites and builds the production image. Releases go through `/deploy:build`
(date tag → `release.yml` builds the image) and `/deploy_app` (`deploy.yml`:
migrate job, then the service), authenticated by Workload Identity Federation.

### Added (v0.3: sign-in providers and sharing)
- Server-side OIDC sign-in through one or more providers at once (slauth, Okta,
  Keycloak, …): `AUTH_PROVIDERS` + `AUTH_<ID>_*`, per-provider allowed email
  domains, verified emails required (or `TRUST_EMAIL`), optional provider
  logout. The browser holds a signed session cookie; no provider tokens are
  stored. A person signing in through two providers with one email is one user.
- Sharing: `private` parcels can be shared with people by email as viewer or
  commenter (even before they first sign in); new `internal` visibility for
  everyone signed in; `PARCEL_ALLOW_PUBLIC=false` turns public links off.
  Unsharing or narrowing visibility stops open pages at once. "Shared with me"
  (`GET /api/v1/shared`). `parcel share`/`unshare`/`shares`, `parcel list
  --shared`, `--internal`.
- Sign out everywhere; `users.disabled_at` blocks a person at once;
  `PARCEL_TOKEN_LOGIN_MAX_AGE_DAYS` pauses API tokens of people who haven't
  signed in on the web lately (so offboarding at the provider reaches them);
  `PARCEL_SESSION_TTL_SECS`.
- A local Keycloak (`docker compose --profile keycloak`) for trying several
  providers.

### Changed (v0.3)
- `AUTH_MODE=slauth` is now `oidc` (the old value still works);
  `SLAUTH_JWKS_URL` and `SLAUTH_AUDIENCE` are no longer read. The API no
  longer accepts slauth access tokens as bearers: the web app uses its
  session cookie, the CLI and agents `pcl_` tokens.
- `/api/v1/info`'s `auth` is now `{mode, providers}`, plus `allow_public`.

### Added (v0.2: comments, live updates, page data)
- Live updates: the viewer polls `/api/v1/parcels/{id}/activity` and reloads
  the page when a new version is published.
- Comments: threads numbered per parcel, optionally pinned to a spot on the
  page from the viewer's comment mode; replies, resolve/reopen, edit, delete.
  The owner, and signed-in users on public parcels, can comment. `parcel
  comments` (list/add/reply/resolve/reopen/delete) for agents.
- Page data: per-parcel JSON collections (`/api/v1/parcels/{id}/data`, and
  `/d/{token}` on the content origin for the page itself). Viewers read; only
  the owner writes. `parcel data` (list/get/set/update/delete/clear).
- The page helper script, added to every HTML page on the content origin:
  in-memory `localStorage`/`sessionStorage`/`document.cookie` so sandboxed
  pages don't throw, the `window.parcel` API, and the comment-mode bridge.
- `PARCEL_MAX_DATA_BYTES` (10 MiB per parcel) and `PARCEL_MAX_DATA_VALUE_BYTES`
  (256 KiB per value).

### Added
- `parcel-server`: slauth-authenticated API for publishing HTML parcels with
  supporting files, versions (merge/replace, optimistic concurrency, restore),
  visibility, configurable size/count/quota limits, a CSP-sandboxed content
  origin behind HMAC capability URLs, Markdown entry pages, content-addressed
  blob storage (filesystem, GCS) with grace-period GC, `pcl_` API tokens and a
  device-flow CLI login. OpenAPI document committed and pinned; Swagger UI in
  development.
- `parcel` CLI: login/logout/whoami, publish (file or folder), list, info, cat,
  pull, update, versions, restore, delete, open, `skill install`.
- The `parcel` agent skill for Claude Code, craze and opencode.
- React web app: dashboard, sandboxed viewer, uploads, tokens, CLI approval.
- Functional (pytest) and Playwright suites; Docker image; Cloud Run deploy
  templates and workflows (prepared, not yet run).

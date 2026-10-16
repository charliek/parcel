# Parcel — MVP plan and decision log

Parcel is a self-hosted take on "artifacts": an agent (or a person) pushes an HTML
page — optionally with supporting files — and gets back a private URL that renders
it in a sandbox. Owners sign in through **slauth**, see the parcels they own, and
manage them from the web app or the `parcel` CLI. A bundled agent **skill** teaches
Claude Code, craze and opencode to publish and read parcels.

This file is the plan of record for the MVP build and the log of the assumptions
made while building it unattended. Anything marked **Assumption** is a judgement
call that is cheap to revisit.

## Scope of the MVP

- Rust backend (`crates/parcel-server`): axum 0.8 + sqlx 0.8 on Postgres, the five
  `stridelabs-*` crates, utoipa OpenAPI with a committed spec, freshness test,
  route-pinning test and Spectral lint; Swagger UI at `/docs` outside production.
- React SPA (`frontend/`): React 19, Vite, Tailwind v4, zustand, react-router v7,
  biome + eslint, bun — the slauth stack. OAuth2 Authorization Code + PKCE against
  slauth, tokens in memory, silent `prompt=none` re-auth (the spendwise pattern).
- CLI (`crates/parcel-cli`, binary `parcel`): login (device flow), publish, list,
  info, cat, pull, update, delete, versions, open, whoami, skill install.
  Packaged for brew/apt the way strix (a Rust CLI) is: `cargo deb` + formula template
  + a release workflow (written, not run — no GitHub yet).
- Skill (`skills/parcel/SKILL.md`), embedded in the CLI with `include_str!` and
  installed by `parcel skill install` into `~/.claude/skills/parcel/` (which Claude
  Code, craze native and opencode all scan). Also a Claude plugin manifest.
- Blob storage behind a `BlobStore` trait: filesystem (dev default), GCS (prod; JSON
  API over reqwest, tested against `fake-gcs-server`), in-memory (tests).
- Deploy artifacts for Cloud Run + Neon + GCS (Dockerfile, service template, docs,
  workflows) — prepared, not executed. The goal for now is **fully working locally**.
- Tests: Rust unit + integration (real Postgres, fail-loud), Python functional tests
  (`tests/functional`, pytest, drive the real binaries over HTTP), and Playwright.

Not in the MVP: the MCP server (designed for, see "Later"), comments, shared
databases, per-viewer storage, teams/sharing with named users.

## Architecture

```
browser ──► parcel.<domain>          SPA + /api/v1/* (JSON API, bearer auth)
        └─► parcelcontent.<domain>   /v/{token}/{path}  sandboxed user content
CLI ──────► parcel.<domain>/api/v1/* (bearer: pcl_… token from device login)
                       │
               parcel-server (one binary, host-based routing)
                 │                 │
             Postgres          BlobStore (fs | gcs)
```

### Content isolation (the security core)

User HTML is untrusted. It is never served from the app origin.

- A separate **content origin** (`PARCEL_CONTENT_ORIGIN`) serves files at
  `/v/{token}/{path}`. The server picks the content router by the request `Host`.
- Every content response carries `Content-Security-Policy: sandbox allow-scripts
  allow-forms allow-popups allow-popups-to-escape-sandbox allow-modals
  allow-downloads; frame-ancestors <app origin>`. The CSP `sandbox` directive gives
  the document an **opaque origin even on a top-level visit**, so a parcel can never
  read or set cookies or storage for the content domain or its parents. This is why
  the content host can live on a sibling subdomain.
- The viewer iframe repeats the sandbox (no `allow-same-origin`).
- `Referrer-Policy: no-referrer` so the capability token in the URL never leaks to
  sites a parcel links to; `Access-Control-Allow-Origin: *` so module scripts,
  fonts and `fetch()` of sibling files work from the opaque origin.
- `{token}` is an HMAC-SHA256 capability (`PARCEL_CONTENT_SIGNING_KEY`) over
  `(parcel id, version, expiry)`. Relative URLs inside a parcel resolve under the
  same token prefix, so multi-file parcels just work with no cookies.
- **Assumption:** `localStorage`/cookies throw inside parcels (opaque origin). The
  skill tells agents to wrap storage in try/catch. Per-parcel subdomains (which would
  allow storage) need wildcard DNS/certs; deferred.

### Data model

`users` (keyed by slauth `sub`) → `parcels` (12-char id, title, description,
visibility `private|public`, current version) → `parcel_versions` (immutable, entry
path, totals) → `version_files` (path → blob sha256, size, content type).
`blobs` are content-addressed (`blobs/sha256/ab/<hash>`) and deduplicated across
versions; unreferenced blobs are garbage-collected after a grace period.
`api_tokens` (`pcl_…`, SHA-256 hashed, revocable) and `device_authorizations`
(CLI device login) round it out.

### Publishing semantics (mirrors Claude artifacts)

- `POST /api/v1/parcels` (multipart: `metadata` JSON + one part per file whose
  filename is the published path) creates a parcel and version 1.
- `POST /api/v1/parcels/{id}/versions` adds a version. `mode=merge` (default):
  files sent are added/replaced, files left out are kept, `remove: [...]` deletes.
  `mode=replace`: the new version is exactly the files sent. Optional
  `base_version` gives optimistic concurrency (409 on mismatch).
- Updating keeps the URL. Old versions remain viewable.
- The entry page must be `.html`/`.htm` or Markdown (`.md`, rendered server-side
  into a styled HTML shell). **Assumption:** other files (css/js/json/images/fonts/
  audio/video/pdf/wasm/…) are supporting files; unknown extensions are rejected.

### Limits (all configurable via env)

| Setting | Default |
|---|---|
| `PARCEL_MAX_ENTRY_BYTES` | 16 MiB |
| `PARCEL_MAX_FILE_BYTES` | 15 MiB |
| `PARCEL_MAX_UPLOAD_BYTES` (one publish) | 64 MiB |
| `PARCEL_MAX_FILES_PER_UPLOAD` | 256 |
| `PARCEL_MAX_FILES_PER_VERSION` | 512 |
| `PARCEL_MAX_VERSION_BYTES` | 256 MiB |
| `PARCEL_USER_QUOTA_BYTES` (sum over all versions; 0 = unlimited) | 2 GiB |

These mirror the published limits of Claude artifacts so skills written for one
port to the other.

### Auth

- **Web:** slauth OAuth2 + PKCE, public client `parcel`, `aud=parcel`. Verified with
  `stridelabs_auth::Verifier`. Users are find-or-created by `sub`.
- **CLI:** **Assumption — device authorization flow** with app-local tokens rather
  than a loopback OAuth client: it works over SSH/remote boxes, needs no new Hydra
  client type, and the slauth registry test forbids loopback redirects for
  first-party clients outside local. `parcel login` → `POST /api/v1/cli/device` →
  user approves the code at `/device` in the web app → CLI polls and receives a
  `pcl_…` token (only its SHA-256 is stored). Tokens can also be minted and revoked
  on the web Tokens page and passed via `PARCEL_TOKEN`.
- **Dev bypass:** `AUTH_MODE=dev` injects a fixed user (for e2e and offline dev). Off
  by default and refused when `APP_ENV=production` (the spendwise guard).

### Visibility

**Assumption:** two levels. `private` (default; owner only) and `public` (anyone with
the link, no sign-in). Named-user sharing is a later feature.

## Environments and domains

| Env | App | Content | Auth |
|---|---|---|---|
| local | `https://parcel.local.stridelabs.ai` (Vite) | `https://parcelcontent.local.stridelabs.ai` | `https://auth.local.stridelabs.ai` |
| staging | `https://parcel-staging.stridelabs.ai` | `https://parcelcontent-staging.stridelabs.ai` | `https://auth-staging.stridelabs.ai` |
| prod | `https://parcel.stridelabs.ai` | `https://parcelcontent.stridelabs.ai` | `https://auth.stridelabs.ai` |

Local ports (chosen to avoid every sibling project): prox API 5563, Vite 5178,
backend 8106, Postgres 5439, fake-gcs-server 4447. `parcelapi.local.stridelabs.ai`
reaches the backend directly.

GCP: project `projects-413401`, region `us-central1`, image
`us-central1-docker.pkg.dev/projects-413401/stridelabs/parcel`, secrets
`{staging|prod}-parcel-{purpose}`, WIF for CI (the tapper pattern), Neon with a
pooled URL for the service and a direct URL for the migrate job.

## slauth registration

`apps.yaml` (app `parcel`, invite-only, empty `mcp_resources`), `hydra/clients.json`
(first-party `parcel` client, redirect `/auth/callback`, post-logout `/`),
`invite_codes.yaml` (a `parcel-…` code; `parcel` added to `early-access-2026`), and
the regenerated `hydra_seed_bodies.json` fixture. Done on a slauth branch
`feature/add-parcel-app` — **not pushed**.

## Later

- **MCP server** at `/mcp` (stateless streamable HTTP, RFC 9728 metadata, separate
  verifier with `aud=https://parcel…/mcp`, scopes `parcel:read`/`parcel:write`) —
  the slaudio design, ported to Rust (likely `rmcp`). The API is shaped so MCP
  tools are thin wrappers over the same service layer.
- Per-parcel origins (wildcard subdomain) to allow storage inside parcels.
- Named-user sharing, comments, per-parcel shared data.
- Publishing the repo to GitHub, then turning on CI / release / deploy workflows.

## HTTP surface (MVP)

App host (`PARCEL_APP_ORIGIN`). JSON unless noted; errors are
`{"error": {"message", "type"}}`. Auth is `Authorization: Bearer <slauth JWT | pcl_ token>`.
Types are in `crates/parcel-types/src/api.rs`.

| Method | Path | Auth | Body → Response |
|---|---|---|---|
| GET | `/health/live`, `/health/ready` | — | plain, not in spec |
| GET | `/openapi.json`; `/docs` (non-prod) | — | spec / Swagger UI |
| GET | `/api/v1/info` | — | `ServerInfo` |
| GET | `/api/v1/me` | required | `Me` |
| GET | `/api/v1/parcels?q=&limit=&offset=` | required | `ParcelList` (owned) |
| POST | `/api/v1/parcels` | required | multipart (`metadata` JSON part = `CreateParcelMetadata`, `file` parts whose *filename* is the published path) → 201 `PublishResult` |
| GET | `/api/v1/parcels/{id}` | owner, or anyone if public | `ParcelDetail` |
| PATCH | `/api/v1/parcels/{id}` | owner | `UpdateParcelRequest` → `ParcelSummary` |
| DELETE | `/api/v1/parcels/{id}` | owner | 204 |
| GET | `/api/v1/parcels/{id}/view?version=N` | owner, or anyone if public | `ViewGrant` (signed content URL) |
| GET | `/api/v1/parcels/{id}/versions` | owner/public | `VersionList` |
| POST | `/api/v1/parcels/{id}/versions` | owner | multipart (`metadata` = `NewVersionMetadata`) → 201 `PublishResult` |
| GET | `/api/v1/parcels/{id}/versions/{version}` | owner/public | `VersionDetail` (`version` = number or `latest`) |
| POST | `/api/v1/parcels/{id}/versions/{version}/restore` | owner | 201 `PublishResult` (new version copying that one) |
| GET | `/api/v1/parcels/{id}/versions/{version}/files/{path}` | owner/public | raw bytes, `Content-Disposition: attachment` |
| GET/POST | `/api/v1/tokens` | required | `TokenList` / `CreateTokenRequest` → 201 `CreatedToken` |
| DELETE | `/api/v1/tokens/{id}` | required | 204 |
| DELETE | `/api/v1/tokens/current` | pcl token | 204 (revokes the calling token: `parcel logout`) |
| POST | `/api/v1/cli/device` | — | `DeviceStartRequest` → `DeviceStartResponse` |
| POST | `/api/v1/cli/device/token` | — | `DevicePollRequest` → `DevicePollResponse` |
| GET | `/api/v1/cli/device/{user_code}` | required | `DeviceInfo` (pending codes only) |
| POST | `/api/v1/cli/device/{user_code}/approve` and `/deny` | required | 204 |
| GET | anything else | — | SPA (`index.html` fallback) when `PARCEL_STATIC_DIR` is set |

Content host (`PARCEL_CONTENT_ORIGIN`): `GET /v/{token}/{path}` serves a file
(`/v/{token}/` redirects to the entry). Nothing else.

Web app routes: `/` (my parcels, or landing when signed out), `/p/:id` and
`/p/:id/v/:version` (viewer; public parcels need no sign-in), `/new` (upload),
`/tokens`, `/cli` (CLI + skill setup), `/device` (approve a CLI login),
`/auth/callback`.

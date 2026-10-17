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

- Rust backend (`crates/parcel-server`): axum 0.8 + sqlx 0.9 on Postgres, the five
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

Not in the MVP: the MCP server (v0.4, see "MCP server"), comments, shared
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
| `PARCEL_MAX_DATA_BYTES` (per parcel: each data item's JSON + 128 B) | 10 MiB |
| `PARCEL_MAX_DATA_VALUE_BYTES` | 256 KiB |

These mirror the published limits of Claude artifacts so skills written for one
port to the other.

### Auth

- **Web:** server-side OIDC sign-in through one or more providers (slauth's public
  client `parcel` by default; Okta, Keycloak… side by side), then a signed session
  cookie. Users are found by `(provider, sub)`, else by email. Since v0.3; the MVP
  had the SPA run slauth's PKCE flow itself. See "Sign-in providers and sharing (v0.3)".
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

Three levels since v0.3: `private` (default; the owner and the people it is
shared with by email), `internal` (anyone signed in) and `public` (anyone with
the link, no sign-in; can be turned off per server). The MVP had only
private and public.

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

`apps.yaml`: app `parcel`, invite-only, scopes `parcel:read` / `parcel:write`
(both `connector_allowed`), and `mcp_resources` (`https://parcel.local.stridelabs.ai/mcp`,
`https://parcel.stridelabs.ai/mcp`; a staging URL only for slauth's seeder).
`hydra/clients.json`: the first-party `parcel` web client (redirect
`/auth/callback`, post-logout `/`) and two MCP connectors, `parcel-claude`
(claude.ai callback) and `parcel-claude-code` (loopback), with
`audience_from_app`. `invite_codes.yaml`: a `parcel-…` code, and `parcel` in
`early-access-2026`.

## MCP server (v0.4)

`<app>/mcp`, on the shared crate stridelabs-mcp (stridelabs-rust; the Rust twin
of stridelabs-python's, on rmcp): stateless Streamable HTTP with JSON
responses, `Host`-checked against the resource, RFC 9728 metadata at
`/.well-known/oauth-protected-resource[/mcp]` naming the MCP provider, and
RFC 6750 challenges. Off unless `PARCEL_MCP_ENABLED`; the provider is
`PARCEL_MCP_PROVIDER` (default `slauth`), its keys `<issuer>/.well-known/jwks.json`.
Supported with slauth: its access tokens carry the same `sub` as its ID
tokens and register the resource as their audience. Okta or Keycloak would
need both checked (Okta's custom servers, for one, put the login in `sub`).
Requests are capped at 16 MiB (rmcp parses a whole request before a tool
runs), and one over 1 MiB, or of unknown length, waits for an upload permit
first, so concurrent publishes stay bounded as on the REST API.

- **Credentials.** The provider's access tokens with `aud` = the resource URL
  (so web-client tokens fail here and MCP tokens fail everywhere else), or a
  `pcl_` token under the REST API's rules (for clients that can only send a
  header). The token is stripped before any tool runs.
- **Who the caller is.** A provider identity seen before is that user (its
  allowed domains re-checked against the token's email if the provider is
  trusted with emails, else the stored one; disabled users refused). Its first use signs
  the person in like the web callback (same email/domain rules); access
  tokens carry no `email_verified`, so that needs the provider's
  `TRUST_EMAIL`, else a web sign-in first. MCP needs no pause rule: the
  provider expires and refreshes its own tokens, so offboarding there
  reaches them.
- **Tools.** `whoami`, `list_parcels` (mine / shared), `get_parcel`,
  `read_file` (≤ 1 MiB; utf8 or base64), `publish_parcel`, `update_parcel`
  (a new version, or title/description/visibility), `restore_version`,
  `delete_parcel`, `list_shares`, `share_parcel`, `unshare_parcel`; comments:
  `list_comments`, `add_comment`, `reply_comment`, `resolve_comments` (with an
  optional reply), `reopen_comment`, `delete_comment`; page data: `list_data`,
  `get_data`, `set_data`, `update_data`, `delete_data`. The CLI has the same
  set, so either is a complete agent interface. Each runs inside
  stridelabs-mcp's `ToolCall` (scope check, timeout, safe errors, one audit
  line on `stridelabs.mcp.tool`) and calls the function its REST handler
  calls, returning the same JSON. `whoami` needs no scope, the other read
  tools `parcel:read`, the rest `parcel:write`.
- **The feedback loop.** The agent publishes a plan or page; people comment in
  the viewer (pinned to elements); when asked, the agent lists the comments,
  revises, and resolves what it addressed with a reply. Comments written
  through MCP are recorded `via` `mcp` (migration 0004) and shown "via MCP".
  Comment bodies come from other people, so `list_comments` says so next to
  them and the server instructions tell agents never to follow instructions
  in them.
- **Claude Code plugins.** `plugins/parcel-mcp` (the server, OAuth client
  `parcel-claude-code` on callback port 3118 — current Claude Code redirects to
  `http://localhost:<port>/callback`, which slauth registers for that port)
  and the root plugin `parcel` (the CLI skill), both in this repo's
  marketplace and released with the CLI.

## Later

- Per-parcel origins (wildcard subdomain) to allow storage inside parcels.
- MCP tools for comments and page data.

## HTTP surface (MVP)

App host (`PARCEL_APP_ORIGIN`). JSON unless noted; errors are
`{"error": {"message", "type"}}`. Auth is the web session cookie or
`Authorization: Bearer <pcl_ token>`. "Viewers" below means whoever the
parcel is open to (the access table in "Sign-in providers and sharing").
Types are in `crates/parcel-types/src/api.rs`.

| Method | Path | Auth | Body → Response |
|---|---|---|---|
| GET | `/health/live`, `/health/ready` | — | plain, not in spec |
| GET | `/auth/login`, `/auth/callback`; POST `/auth/logout` | — | browser sign-in redirects, not in spec (see v0.3) |
| GET | `/openapi.json`; `/docs` (non-prod) | — | spec / Swagger UI |
| GET | `/api/v1/info` | — | `ServerInfo` |
| GET | `/api/v1/me` | required | `Me` |
| GET | `/api/v1/parcels?q=&limit=&offset=` | required | `ParcelList` (owned) |
| POST | `/api/v1/parcels` | required | multipart (`metadata` JSON part = `CreateParcelMetadata`, `file` parts whose *filename* is the published path) → 201 `PublishResult` |
| GET | `/api/v1/parcels/{id}` | viewers | `ParcelDetail` (with `access`, `owner_name`) |
| PATCH | `/api/v1/parcels/{id}` | owner | `UpdateParcelRequest` → `ParcelSummary` |
| DELETE | `/api/v1/parcels/{id}` | owner | 204 |
| GET | `/api/v1/parcels/{id}/view?version=N` | viewers | `ViewGrant` (signed content URL) |
| GET | `/api/v1/parcels/{id}/versions` | viewers | `VersionList` |
| POST | `/api/v1/parcels/{id}/versions` | owner | multipart (`metadata` = `NewVersionMetadata`) → 201 `PublishResult` |
| GET | `/api/v1/parcels/{id}/versions/{version}` | viewers | `VersionDetail` (`version` = number or `latest`) |
| POST | `/api/v1/parcels/{id}/versions/{version}/restore` | owner | 201 `PublishResult` (new version copying that one) |
| GET | `/api/v1/parcels/{id}/versions/{version}/files/{path}` | viewers | raw bytes, `Content-Disposition: attachment` |
| GET/POST | `/api/v1/tokens` | required | `TokenList` / `CreateTokenRequest` → 201 `CreatedToken` |
| DELETE | `/api/v1/tokens/{id}` | required | 204 |
| DELETE | `/api/v1/tokens/current` | pcl token | 204 (revokes the calling token: `parcel logout`) |
| POST | `/api/v1/cli/device` | — | `DeviceStartRequest` → `DeviceStartResponse` |
| POST | `/api/v1/cli/device/token` | — | `DevicePollRequest` → `DevicePollResponse` |
| GET | `/api/v1/cli/device/{user_code}` | required | `DeviceInfo` (pending codes only) |
| POST | `/api/v1/cli/device/{user_code}/approve` and `/deny` | required | 204 |
| GET | `/api/v1/parcels/{id}/activity` | viewers | `ParcelActivity` (the viewer's poll; `no-store`) |
| GET/POST | `/api/v1/parcels/{id}/comments?status=open\|resolved\|all` | signed-in viewers / commenters | `CommentList` / `NewComment` → 201 `Comment` (the thread) |
| PATCH/DELETE | `/api/v1/parcels/{id}/comments/{number}` | author / owner (see below) | `UpdateComment` → `Comment` / 204 |
| GET | `/api/v1/parcels/{id}/data` | viewers | `DataSummary` |
| GET/DELETE | `/api/v1/parcels/{id}/data/{collection}?limit=&after=` | read: viewers; delete: owner | `DataItemList` / 204 |
| GET/PUT/PATCH/DELETE | `/api/v1/parcels/{id}/data/{collection}/{key}` | read: viewers; write: owner | any JSON → `DataItem` (PATCH = RFC 7386 merge) / 204 |
| GET | `/api/v1/parcels/{id}/shares` | owner | `ShareList` |
| PUT/DELETE | `/api/v1/parcels/{id}/shares/{email}` | owner | `PutShareRequest` → `Share` / 204 |
| GET | `/api/v1/shared?limit=&offset=` | required | `SharedParcelList` (shared with me by email) |
| GET | anything else | — | SPA (`index.html` fallback) when `PARCEL_STATIC_DIR` is set |

Content host (`PARCEL_CONTENT_ORIGIN`): `GET /v/{token}/{path}` serves a file
(`/v/{token}/` redirects to the entry); `/d/{token}[/{collection}[/{key}]]`
is the page's data (same verbs and bodies as the API above; writes need an
owner-scope token; CORS `*`, `OPTIONS` preflight); `/_parcel/helper.<hash>.js`
is the page helper. Nothing else.

Web app routes: `/` (my parcels and "shared with me", or landing when signed
out), `/p/:id` and `/p/:id/v/:version` (viewer; public parcels need no
sign-in), `/new` (upload), `/tokens`, `/cli` (CLI + skill setup), `/device`
(approve a CLI login), `/signin` (pick a provider; sign-in errors).

## Comments, live updates and page data (v0.2)

**Live updates.** The viewer polls `GET /parcels/{id}/activity` every ~5 s
while its tab is visible (paused when hidden). A new `current_version`
reloads the frame when the viewer shows the current version; when it shows an
older version or the user is writing a comment, a banner offers the reload
instead. A changed `comments_revision` refetches comments. Polling (not SSE)
works across Cloud Run instances with no shared bus; the endpoint is one
indexed row.

**Comments.** Threads are numbered per parcel (`#1`, `#2`, …; replies take
numbers too, numbers are never reused) and may carry an anchor
`{path, selector?, text?, x?, y?}`: the page, a CSS selector of the clicked
element, its text (to re-find it after edits), and the click position inside
its box. Who may: read/write — the owner, plus any signed-in user when the
parcel is public (anonymous viewers see none); edit — the author; resolve or
reopen a thread — the owner or the thread's author; delete — the author or
the owner. `via` is `web` or `cli` (a `pcl_` token). A parcel holds at most
2000 comments.

**Page helper.** Every HTML response on the content origin (HTML files and
rendered Markdown entries) gets `<script src="/_parcel/helper.<hash>.js">`
right after `<head>` (else after `<html>`, else after the doctype). The ETag
includes the hash. The script (`crates/parcel-server/assets/parcel-helper.js`):

1. replaces `localStorage`, `sessionStorage` and `document.cookie` with
   in-memory stand-ins when they throw (they always do in the sandbox);
2. exposes `window.parcel = {id, version, path, canWrite, data}` where `data`
   has `get, list, set, update, delete, clear, collections, subscribe`,
   calling `/d/{token}/…` with the page's own capability;
3. when framed, speaks a postMessage protocol with the viewer, only with
   `PARCEL_APP_ORIGIN` (checked both ways; the origin is baked into the
   script at startup):

| Direction | `type` | Fields |
|---|---|---|
| viewer → page | `hello` | — (page answers `ready`) |
| viewer → page | `comment-mode` | `enabled: bool` |
| viewer → page | `pins` | `pins: [{number, anchor, resolved, active}]` (the full set for the page; the one flagged `active` is highlighted) |
| viewer → page | `draft` | `anchor \| null` (the spot being commented on) |
| viewer → page | `focus` | `number` (scroll to and pulse that pin) |
| page → viewer | `ready` | `path, title, version, token` (on load and on `hello`; `token` is the page's own capability token, read from its URL) |
| page → viewer | `anchor` | `anchor` (a click in comment mode) |
| page → viewer | `pin-click` | `number` |
| page → viewer | `located` | `found: number[], missing: number[]` |
| page → viewer | `escape` | — (Escape pressed in comment mode) |
| page → viewer | `bye` | — (`pagehide`: the viewer stops sending until the next verified `ready`) |

Viewer messages carry `source: 'parcel-viewer'`, page messages
`source: 'parcel-helper'`. The viewer accepts page messages only from its
iframe's `contentWindow` and treats them as untrusted input (the page can
forge any of them); it sends anchors and numbers, never comment text. The
frame can be navigated to another site that allows framing, which posts from
the same `"null"` origin, so the bridge goes live only after a `ready` whose
`token` equals the one in the URL the viewer loaded, and drops back on every
iframe `load` (then `hello`), `bye` or remount; until then the viewer sends
only `hello` and acts on nothing but `ready`/`bye`. Framed, the helper opens
links to other origins in a new tab.

**Page data.** Collections of keyed JSON items per parcel, shared by every
version, deleted with the parcel. Readable by anyone who can view the parcel;
writable only by its owner — through the API (CLI, agents) or from the page
with an owner-scope capability (minted only for the owner's own viewing).
Names are `[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}`. Limits: one value
`PARCEL_MAX_DATA_VALUE_BYTES` (256 KiB); a parcel `PARCEL_MAX_DATA_BYTES`
(10 MiB), counting each item's compact JSON plus 128 bytes. Visitor writes
(polls from anonymous viewers) are deliberately not supported yet.

## Sign-in providers and sharing (v0.3)

Goal: one Parcel can sign people in through several OIDC providers at once
(slauth for personal use; Okta and Keycloak side by side for offices on
different email domains), and parcels can be shared with named people or
with everyone signed in. Keep server state minimal: low volume is assumed.

**Sign-in is server-side.** The server is the OIDC client (authorization
code + PKCE + nonce, `client_secret_basic`/`_post` when a secret is set,
public client otherwise). It reads the provider's discovery document
(`{issuer}/.well-known/openid-configuration`, whose `issuer` must match
exactly), verifies the **ID token** (signature via the provider's JWKS,
`iss`, `aud` ∋ client id, `azp` when several audiences, `exp`, `nonce`), and
falls back to `userinfo` (same `sub`) when the ID token has no email. ID
tokens are the standardized part of OIDC, so no provider needs custom
access-token setup (Okta's org server works; Keycloak needs no audience
mapper). The browser never holds a provider token, and none are stored.

| Endpoint (app host, outside `/api/v1`) | What |
|---|---|
| `GET /auth/login?provider=&return_to=&login_hint=` | 303 to the provider. Sets a signed, 10-minute `__Host-parcel_login` cookie (`parcel_login` on http dev origins; `SameSite=Lax`; `__Host-` so a sibling subdomain can't plant one) holding provider, state, nonce, PKCE verifier and `return_to` (a same-site path). `provider` may be omitted when only one is configured. |
| `GET /auth/callback?code=&state=` | Checks the cookie and state, redeems the code, applies the provider's policy, signs the user in, 303 to `return_to`. Failures 303 to `/signin?error=<code>` (`denied`, `expired`, `domain`, `unverified`, `failed`, `unavailable` — provider unreachable, `disabled`). |
| `POST /auth/logout[?everywhere=true]` | Clears the session; `everywhere` also ends every other session of the user. Returns `{"redirect"}`: the provider's `end_session_endpoint` (with the ID token hint, kept in an `HttpOnly` cookie scoped to `Path=/auth/logout`) or `/`. |

**Sessions are a signed cookie, not a table.** `__Host-parcel_session`
(`parcel_session` on plain-http dev origins), `HttpOnly; Secure; SameSite=Lax;
Path=/`, value = HMAC-signed `{user_id}.{epoch}.{issued}.{expires}.{provider}`
under a key derived from `PARCEL_CONTENT_SIGNING_KEY` (domain-separated, so
no new secret). Each request checks the signature, expiry, that the user
exists and isn't disabled, and that `users.session_epoch` still matches
(bumped by "sign out everywhere"). Lifetime `PARCEL_SESSION_TTL_SECS`
(default 7 days); after it the provider is asked again (silently, when its
own session is alive). A bad or expired cookie is a 401 that also clears it,
never a silent downgrade to anonymous. The cost of keeping no session rows:
plain sign-out only clears the browser's cookie, so a copied cookie keeps
working until it expires or the user signs out everywhere.

**CSRF.** A cookie only counts when `Sec-Fetch-Site` is absent, `same-origin`
or `none`; unsafe methods (anything but GET/HEAD/OPTIONS) authenticated by
cookie also need `Origin` = `PARCEL_APP_ORIGIN`. Bearer `pcl_` tokens (CLI,
agents) are unaffected. The content origin is sandboxed (opaque origin), so
its requests never carry the app's cookie.

**Providers and policy (config).**

| Var | Meaning |
|---|---|
| `AUTH_MODE` | `oidc` (default; `slauth` is accepted as an alias) or `dev` |
| `AUTH_PROVIDERS` | comma list of ids (`[a-z][a-z0-9]{0,31}`), e.g. `okta,keycloak`; each reads `AUTH_<ID>_*` below |
| `AUTH_<ID>_ISSUER`, `AUTH_<ID>_CLIENT_ID` | required |
| `AUTH_<ID>_CLIENT_SECRET` | optional (absent = public client + PKCE) |
| `AUTH_<ID>_LABEL` | button text (default: the id) |
| `AUTH_<ID>_ALLOWED_DOMAINS` | comma list of email domains this provider may sign in; empty = any |
| `AUTH_<ID>_TRUST_EMAIL` | `true` accepts emails the provider doesn't mark `email_verified` (default `false`) |
| `AUTH_<ID>_SCOPES` | default `openid email profile` |
| `PARCEL_SESSION_TTL_SECS` | default 604800 (7 days) |
| `PARCEL_TOKEN_LOGIN_MAX_AGE_DAYS` | a `pcl_` token stops working when its owner hasn't signed in on the web for this long (default 30; 0 = never) — offboarding at the provider then reaches CLI tokens too |
| `PARCEL_ALLOW_PUBLIC` | `false` turns off public links: setting `public` is refused and existing public parcels behave (and report) as `internal` until it's turned back on |

Unset `AUTH_PROVIDERS` means one provider, `slauth`, from the existing
`SLAUTH_ISSUER` / `SLAUTH_CLIENT_ID` with `TRUST_EMAIL` on (slauth issues no
`email_verified`; its accounts are invite-only). `SLAUTH_JWKS_URL` and
`SLAUTH_AUDIENCE` are no longer read. slauth's registered client (public,
redirect `/auth/callback`, post-logout `/`) works unchanged.

**Identities.** `user_identities (provider, subject) → user`. Every sign-in
needs a trusted email (verified, or `TRUST_EMAIL`) in an allowed domain. A
new identity links to the existing user with the same email (that's what
makes a person one user across providers), else creates one. Existing users
migrate as `slauth` identities (`dev:` ones as `dev`). `users.disabled_at`
(set by an operator in SQL for now) blocks sessions and tokens at once.

**Visibility.** `private` (owner and the people it's shared with),
`internal` (anyone signed in to this Parcel), `public` (anyone with the
link). Anonymous callers get 404 for private and internal parcels.

| Caller | private | internal | public |
|---|---|---|---|
| owner | owner | owner | owner |
| shared with (by email) | its role | commenter | commenter |
| any other signed-in user | 404 | commenter | commenter |
| anonymous | 404 | 404 | view (no comments) |

Roles: `viewer` sees the parcel, its comments and its data; `commenter`
also comments. Page data stays owner-write-only (invariant 8).

**Shares** are `(parcel_id, email, role)`, matched against the signed-in
user's (trusted) email at request time — so sharing with someone who hasn't
signed in yet just works on their first sign-in, with no pending-invite
state. At most 200 per parcel. An email no configured provider would accept
(every provider restricts domains and none lists this one) is refused.

**Content tokens** gain scope `m` (a signed-in non-owner) carrying the user
id: `{id}.{version}.{exp}.m.{user_id}`. Every content and `/d/` request
with it re-checks that the user still has access, as `p` re-checks that the
parcel is still (effectively) public — so unsharing, going private, or
turning public off cuts off open pages immediately.

**API additions** (all in the spec): `GET /api/v1/parcels/{id}/shares` →
`ShareList`; `PUT`/`DELETE /api/v1/parcels/{id}/shares/{email}` (`{role}` →
`Share` / 204), owner only; `GET /api/v1/shared` → `SharedParcelList`
(parcels shared with me by name). `ParcelDetail` and `ViewGrant` gain
`access` (`owner|commenter|viewer|public`) and `owner_name` (signed-in
callers). `ServerInfo.auth` becomes `{mode, providers: [{id, label}]}` plus
`ServerInfo.allow_public`; `Me` gains `signed_in_with` (the provider label).

After a provider refuses someone (wrong domain), its own session may still
be signed in, so choosing it again refuses again until they sign out there.

Not now (ROADMAP): groups from provider claims, an editor role, invite
emails/notifications, an admin UI (disable users, see sign-ins), SCIM,
discovery of internal parcels.

## Status (2026-10-16)

The MVP is built and verified locally, end to end:

| Check | Result |
|---|---|
| `cargo test --workspace` (real Postgres per-test schemas, GCS emulator) | 116 passed |
| `pytest tests/functional` (real server + CLI binaries, GCS emulator, test JWKS, real device-flow login through the CLI) | 14 passed |
| `bun test` (frontend units) / Playwright e2e against the production build (`scripts/e2e-stack.sh`) | 75 / 17 passed |
| clippy `-D warnings` (default + production features), fmt, Spectral, biome/eslint, tsc | clean |
| Real slauth login in a browser (playwright-cli) → dashboard → CLI device login approved in the web app → CLI publish → sandboxed viewer → public share viewed signed-out | works |
| Production Docker image (distroless, 53 MB) run with `APP_ENV=production` + GCS emulator | works |
| Claude Code headless with only the project-installed skill: wrote and published a page, then revised it by URL (v2, same link) | works; opencode discovers the skill |

An independent Codex review found 11 issues (GC/publish races, duplicate-blob
upserts, device-redeem atomicity, `parcel pull` symlink handling, token leaks
in errors/logs, cache revocation); all are fixed with regression tests where
testable.

### v0.2 (comments, live updates, page data)

| Check | Result |
|---|---|
| `cargo test --workspace` | 156 passed |
| `pytest tests/functional` (comments, data, helper injection, CLI flows) | 20 passed |
| `bun test` / Playwright e2e on the production build | 135 / 27 passed (incl. live update, comment mode, frame-bridge spoofing, the reload-vs-draft race, page data) |
| Browser walkthrough (playwright-cli): a dashboard reading `parcel.data` updates live from `parcel data update`; comment pinned in comment mode; `parcel comments list` shows the anchor; a v2 publish reloads the open viewer and the pin re-finds its element; `parcel comments resolve --reply` shows up in the viewer; phone width | works |
| Helper in real Chromium: storage stand-ins, `parcel.data`, `<base href>` pages, pages that wrap `fetch`, injection past comments/quoted attributes; `/d/<token>` redacted in logs | works |

A Codex review of v0.2 found 6 issues, all fixed: data calls resolved
against a page's `<base>` (capability leak) and went through a page-wrapped
`fetch`; `/d/` capabilities were not redacted in logs (nor in the documented
Cloud Run log exclusion); delete-vs-write lock order could deadlock (every
data/comment change now locks the parcel row first); injection could land
inside a comment or quoted attribute; control characters in comments reached
terminals via `parcel comments list` (now refused by the server and escaped
by the CLI); a slow auto-reload could swap the frame under a comment draft
(now decided when the grant arrives, and drafts keep their version). Earlier,
the frame bridge was hardened so a document navigated into the viewer's
iframe can't pose as the parcel (token-verified `ready`, `bye` on unload,
external links open in a new tab).

### v0.3 (sign-in providers and sharing)

| Check | Result |
|---|---|
| `cargo test --workspace` | 216 passed (20 sign-in tests against a mock provider, incl. five signature forgeries; 7 sharing/visibility) |
| `pytest tests/functional` (in-process mock OIDC provider with two tenants) | 36 passed |
| `bun test` / Playwright e2e on the production build | 160 / 43 passed |
| clippy (both feature sets), fmt, Spectral, biome/eslint, tsc | clean |
| Real Keycloak 26 (`dev/keycloak`), scripted | confidential client + PKCE + `iss` check; verified user signs in; unverified email → `unverified`; other domain → `domain` |
| Browser walkthrough, slauth **and** Keycloak side by side (playwright-cli) | provider picker; Keycloak user publishes a private parcel and shares it by email with the slauth user as commenter; the slauth user finds it under "Shared with me" and pins a comment; downgraded to viewer, comments go read-only; unshared, their open viewer turns into "not found" within one poll; made internal, they can open and comment while anonymous callers get 404; a refused sign-in names the provider |
| Existing dev database migrated (slauth + dev users become identities) | works |

A Codex review of v0.3 found 8 issues, all fixed with tests: the login
cookie could be planted from a sibling subdomain (now `__Host-`); a
disabled owner's open page kept owner authority over files and data; an
https issuer could advertise plain-http endpoints (now refused, and OIDC
calls never follow redirects); an ended session could still "sign out
everywhere"; concurrent first sign-ins could split one email into two users
(now serialized per email); a stale logout hint could reach another
provider; the migration dropped `users.sub` under a still-running old
revision (now kept, nullable); and signature forgery wasn't tested. Not
verified live: Okta (no account here; its setup is in `deploy/README.md`).

Deviations from the original brief, by design:
- CLI auth is a device flow + app-local `pcl_` tokens rather than slauth PATs or
  a loopback OAuth client (see "Auth"). Accepting slauth PATs too is on the roadmap.
- Nothing is deployed: Cloud Run/Neon/GCS/DNS setup is scripted and documented in
  `deploy/README.md` but not executed; GitHub workflows are written but not run.
- slauth registration lives on slauth's local `feature/add-parcel-app` branch
  (committed, not pushed), which is currently checked out in `../slauth`.

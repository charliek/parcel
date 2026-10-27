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
- Blob storage behind a `BlobStore` trait: filesystem (dev default), GCS (prod;
  `stridelabs-gcp`'s JSON-API client since plan 002, tested against
  `fake-gcs-server` and wiremock), in-memory (tests). See "Blob storage" below.
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

`users` (one per person; `user_identities` maps each provider `(provider, subject)` to one) → `parcels` (12-char id, title, description,
visibility `private|public`, current version) → `parcel_versions` (immutable, entry
path, totals) → `version_files` (path → blob sha256, size, content type).
`blobs` are content-addressed (`blobs/sha256/ab/<hash>`) and deduplicated across
versions; unreferenced blobs are garbage-collected after a grace period.
`api_tokens` (`pcl_…`, SHA-256 hashed, revocable) and `device_authorizations`
(CLI device login) round it out. v0.5 adds tags, archiving, full-text search,
handles, slugs, version sources and page titles; see "Tags, slugs, archive and
search (v0.5)".

### Publishing semantics (mirrors Claude artifacts)

- `POST /api/v1/parcels` (multipart: `metadata` JSON + one part per file whose
  filename is the published path) creates a parcel and version 1.
- `POST /api/v1/parcels/{id}/versions` adds a version. `mode=merge` (default):
  files sent are added/replaced, files left out are kept, `remove: [...]` deletes.
  `mode=replace`: the new version is exactly the files sent. Optional
  `base_version` gives optimistic concurrency (409 on mismatch).
- A publish's transaction (create, slug upsert, new version, restore), a
  PATCH and a tag rename or delete rerun once on a deadlock (40P01) or
  serialization failure (40001), after 10–50 ms, logged with the SQLSTATE
  only (`db::retry`). Blobs are written before the transaction and not
  again; the rerun re-reads everything under the locks, so a stale
  `base_version` is still a 409.
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

### Blob storage

The GCS backend (`storage/gcs.rs`) is a `BlobStore` adapter over
`stridelabs-gcp`'s `GcsClient`; parcel keeps only the policy:

- **Names:** `GCS_PREFIX` with its outer slashes trimmed (nothing else), `/`,
  then the blob key, sent as GCS addresses it (`ObjectNamePolicy::Literal`), so
  objects written before the library stay reachable under any prefix.
- **Endpoint and credentials, chosen independently:** `STORAGE_EMULATOR_HOST`
  (scheme defaulting to `http://`, trailing slashes dropped) picks the emulator,
  otherwise the JSON API; `GOOGLE_CREDENTIALS` (strict base64 service-account
  JSON; a missing `type` is taken as `service_account`, `token_uri` is required,
  the scope is `devstorage.read_write`, no `x-goog-user-project`) picks the key,
  otherwise the emulator goes unauthenticated and the JSON API uses the metadata
  server (no `?scopes=`, 10 s timeout). A blank `GOOGLE_CREDENTIALS` is unset,
  like every variable. A key that is set is checked at startup
  (`ServiceAccountKey::validate`: it signs, and its `token_uri` is https or
  loopback http with no userinfo, query or fragment), as is `GCS_BUCKET`'s name;
  a bad one stops the server, with an error that never quotes the value. Never a
  fallback to another identity, and no process-environment credential lookup.
- **Two clients:** the writer (`put`, `exists`, `delete`) has 60 s per attempt, at
  most 3 attempts, retrying only within 30 s of the first attempt's start; the
  reader (`get`, viewing and indexing) 30 s, 2 attempts, within 10 s. Both retry
  408/429/500/502/503/504, transient transport or token failures and attempts cut
  off by their timeout, on Google's backoff (1 s doubling), never 401/403/404. A
  retry starts only if its backoff ends within the deadline, and each deadline is
  shorter than its client's timeout, so a timed-out attempt is never retried while
  quick failures are. Worst cases: a viewer waits about 10 s of quick retries plus
  one 30 s attempt, a write (or a GC delete, which holds its blob row's lock
  meanwhile) about 30 s plus one 60 s attempt.
- **Read cap:** a download stops at `max(max(PARCEL_MAX_ENTRY_BYTES,
  PARCEL_MAX_FILE_BYTES), PARCEL_MAX_UPLOAD_BYTES, 64 MiB)` bytes. No blob is
  larger than the limits in force when it was stored, so a stored blob becomes
  unreadable only if its size exceeds 64 MiB *and* every current one of
  `PARCEL_MAX_ENTRY_BYTES`, `PARCEL_MAX_FILE_BYTES` and `PARCEL_MAX_UPLOAD_BYTES`
  (e.g. 100 MiB stored under 128 MiB limits, then all three lowered to 96 MiB).
  Raising the limits again makes it readable.
- **Errors** are the library's: an operation (`upload`, `download`, `exists`,
  `delete`), a status and Google's plain `error.status` code; never an object
  name, Google's free-text message, a bearer or key material, and no URL beyond a
  token endpoint's origin (`scheme://host[:port]`).

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

### Log caps

Some dependencies log user content at levels production doesn't run but an
operator chasing a problem might set (`RUST_LOG=debug`). `log_cap_list()`
(`lib.rs`) caps those targets whatever `RUST_LOG` says: stridelabs-observability's
`TargetCaps`, a second filter beside the level filter that can only remove,
where every cap covering a target applies and the strictest wins. `main.rs`
installs them with `try_init_logging_with` and refuses to start if it can't.

- **sqlx** (stridelabs-postgres's `LOG_CAPS`, all off): a raw `sqlx::Error`
  quotes the value the server rejected; also server notices and URL values.
- **`rmcp` at ERROR.** Below it rmcp logs each MCP request and result whole
  (tool arguments, page HTML, file contents, comment text, page data, and
  the serde error that quotes a malformed argument) and, at TRACE, every
  transport message. Its ERROR lines are transport and task failures (a
  task's panic message included; none on parcel's path carries input).
- **`rmcp::transport::worker` and `rmcp::transport::streamable_http_server::tower`
  off:** their ERROR lines can quote a client message, on session paths the
  stateless transport doesn't take. Re-audit rmcp's log sites on each bump.

The cost: rmcp's own diagnostics below ERROR are gone (drain timeouts,
notification and response send failures, rejected requests, its per-request
INFO lines), and a `tools/call` with malformed arguments leaves only the HTTP
access line: rmcp answers it with an `isError` result before any tool runs,
so there is no audit line or diagnostic either. Parcel's MCP auth decisions
and tool outcomes (`stridelabs.mcp.tool`) still log. A value-free diagnostic
for malformed arguments is future work. `tests/mcp_logs.rs` runs `/mcp` at
`RUST_LOG=trace` with and without the caps.

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
  header). The token is stripped before any tool runs. A refusal is `401
  invalid_token` (re-authorize) only when the token itself is bad; when the
  check can't be made it is `503` (retry with the same token): the
  provider's JWKS unreachable or its cached set past its TTL (never served
  stale), or a `kid` the cache has never seen inside its 30 s refetch floor
  (possibly a key rotation not fetched yet; a retired key's `kid` is a 401).
- **Who the caller is.** A provider identity seen before is that user (its
  allowed domains re-checked against the token's email if the provider is
  trusted with emails, else the stored one; disabled users refused). Its first use signs
  the person in like the web callback (same email/domain rules); access
  tokens carry no `email_verified`, so that needs the provider's
  `TRUST_EMAIL`, else a web sign-in first. MCP needs no pause rule: the
  provider expires and refreshes its own tokens, so offboarding there
  reaches them.
- **Tools** (25). `whoami`, `list_parcels` (mine / shared / all; `query` is
  full text, `tags` all-of, `archived`), `list_tags`, `rename_tag`,
  `delete_tag`, `get_parcel` (with `pages`),
  `read_file` (≤ 1 MiB; utf8 or base64), `publish_parcel` (`tags`, `slug` =
  upsert, `created` in the result), `update_parcel`
  (a new version with `add_tags`, or title/description/visibility/tags/slug/
  archived), `restore_version`,
  `delete_parcel`, `list_shares`, `share_parcel`, `unshare_parcel`; comments:
  `list_comments`, `add_comment`, `reply_comment`, `resolve_comments` (with an
  optional reply), `reopen_comment`, `delete_comment`; page data: `list_data`,
  `get_data`, `set_data`, `update_data`, `delete_data`. The CLI has the same
  set, so either is a complete agent interface. Each runs inside
  stridelabs-mcp's `ToolCall` (scope check, timeout, safe errors, one audit
  line on `stridelabs.mcp.tool`) and calls the function its REST handler
  calls, returning the same JSON. `whoami` needs no scope, the other read
  tools `parcel:read`, the rest `parcel:write` (`rename_tag`, `delete_tag`
  included). Every `parcel` argument takes an id, a URL, `@handle/slug` or a
  bare slug; a slug goes through the by-slug resolver, so a private parcel and
  a missing one give the same error, and a ref's `?v=`/`?page=` are defaults
  that explicit `version`/`path` arguments override. List output stays a
  superset: `mine`/`all` items are the flattened summary (`shared` on others'),
  `shared` items keep `{parcel, owner_name, role, shared_at}` (plus `match`).
  The server instructions teach finding and organizing: reuse `list_tags`,
  tag `project:`/`kind:`, slug plans, search before writing.
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

Tracked as GitHub issues (see ROADMAP.md), e.g. per-parcel origins to allow
storage inside parcels (#12).

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
| GET | `/api/v1/info` | — | `ServerInfo` (with `features`: optional API features such as `tags`, `search`, `archive`, `slugs`) |
| GET | `/api/v1/me` | required | `Me` (with `handle`) |
| PATCH | `/api/v1/me` | required | `UpdateMeRequest` `{handle}` → `Me`; 400 malformed, 409 taken (by anyone, current or retired); the old handle stays yours and keeps resolving |
| GET | `/api/v1/parcels?scope=&tag=&tag=&archived=&q=&limit=&offset=` | required | `ParcelList` of `ParcelListItem` (summary fields flattened, plus `match` snippet segments and `shared`); `scope` = `mine` (default) / `shared` / `all`, tags AND, `archived` = `exclude` (default) / `include` / `only`, `q` = substring or full text, ranked (all words; else any word, with `relaxed: true`) |
| POST | `/api/v1/parcels` | required | multipart (`metadata` JSON part = `CreateParcelMetadata`, `file` parts whose *filename* is the published path) → 201 `PublishResult`; with a `slug` you already use (or a redirect of yours), 200 and a new version of that parcel (`created: false`; see "Handles, slugs and sources") |
| GET | `/api/v1/parcels/by-slug?handle=&slug=` | viewers | `ParcelDetail` of the parcel `/@<handle>/<slug>` names (old handles and slugs too; `parcel.canonical_url` is the current one); every miss is the same 404; `handle` defaults to yours, 400 without one when signed out |
| GET | `/api/v1/parcels/{id}` | viewers | `ParcelDetail` (with `access`, `owner_name`, `pages`) |
| PATCH | `/api/v1/parcels/{id}` | owner | `UpdateParcelRequest` → `ParcelSummary` (`tags` replace or `add_tags`/`remove_tags`, `archived`, `slug` (`""` clears; 409 if another of yours has it); none of these bumps `updated_at`) |
| DELETE | `/api/v1/parcels/{id}` | owner | 204 |
| GET | `/api/v1/parcels/{id}/view?version=N&path=P` | viewers | `ViewGrant` (signed content URL of the entry, or of page `P`; 404 if `P` isn't a page of that version) |
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
| GET | `/api/v1/shared?limit=&offset=` | required | `SharedParcelList` (shared with me by email; legacy, unchanged: archived included) |
| GET | `/api/v1/tags?scope=&tag=&q=&mode=&archived=&prefix=&limit=&offset=` | required | `TagList` (facet counts over the parcels the same filters select, `q` in `mode` `auto`/`exact`/`relaxed`; `relaxed` when the relaxed search ran; limit ≤ 1000) |
| POST | `/api/v1/tags/rename` | required | `RenameTagRequest` → `TagsUpdated` (the caller's parcels; merges) |
| POST | `/api/v1/tags/delete` | required | `DeleteTagRequest` → `TagsUpdated` (the caller's parcels) |
| GET | anything else | — | SPA (`index.html` fallback) when `PARCEL_STATIC_DIR` is set |

Content host (`PARCEL_CONTENT_ORIGIN`): `GET /v/{token}/{path}` serves a file
(`/v/{token}/` redirects to the entry); `/d/{token}[/{collection}[/{key}]]`
is the page's data (same verbs and bodies as the API above; writes need an
owner-scope token; CORS `*`, `OPTIONS` preflight); `/_parcel/helper.<hash>.js`
is the page helper. Nothing else.

Web app routes: `/` (my parcels and "shared with me", or landing when signed
out), `/p/:id` and `/p/:id/v/:version` (viewer; public parcels need no
sign-in), `/@:handle/<slug>` (the same viewer by clean URL), `/new` (upload),
`/tags` (rename, merge and delete tags), `/tokens`, `/cli` (CLI + skill setup),
`/device` (approve a CLI login), `/signin` (pick a provider; sign-in errors).
`/` takes `?scope=mine|shared|all&tag=&tag=&q=&archived=` (`?tab=shared` still
works); a viewer URL takes `?page=<path>`.

## Tags, slugs, archive and search (v0.5)

Finding and organizing parcels once there are many, and a stable name for
the ones an agent republishes. Three additive migrations (and a later
guard on the search trigger, 0009); the pieces that need more than a
paragraph have their own sections ("Handles, slugs and sources", "Pages
within a parcel").

| Migration | Adds |
|---|---|
| 0006 | `parcels.tags TEXT[]` (GIN), `archived_at`, `search_text`, `search_vec` (GIN), `search_attempts`; the triggers that keep the search columns right |
| 0007 | `user_handles` + `parcel_allocate_handle()`, `parcels.slug` (partial unique index per owner), `parcel_slug_redirects`, `parcel_versions.source` (JSONB) |
| 0008 | `version_files.page_title` |
| 0009 | `parcels_search_vec()` keeps the vector when an UPDATE leaves title, description, tags and page text as they were |

**Rollout safety.** A Cloud Run rollover runs the old and the new revision
side by side, so every migration is additive (new columns nullable or
defaulted, new state in new tables) and rolling the code back is safe. What an
older revision can't know is done in the database: a trigger clears
`search_text` and `archived_at` when `current_version` changes, so a publish
or restore from the old revision still invalidates the page text and
un-archives. Handles are allocated by one SQL function, lazily for a user an
older revision created. Clients feature-check: `ServerInfo.features` lists
`tags`, `search`, `archive`, `slugs`, `pages`, `search-relaxed`, and the CLI refuses a new
parameter an old server would silently ignore.

### Tags

- Free-form, up to 20 per parcel, visible to everyone who can view it.
  Normalized on every write by `parcel-types` (the SPA has a tested twin):
  trimmed, lowercased, whitespace and `_` → `-`, then
  `^[a-z0-9][a-z0-9.+-]*(:[a-z0-9][a-z0-9.+/-]*)?$`, ≤ 64 characters. A bad
  tag is a 400 naming it. Stored sorted by bytes; SQL that rewrites arrays
  sorts `COLLATE "C"` so both writers agree.
- **Conventions, not rules.** `project:<name>` (the repository),
  `kind:<kind>` with the canonical kinds `plan`, `discovery`, `research`,
  `report`, `review`, `dashboard`, `prototype`, `notes`; any other `ns:value`
  is a facet of its own and bare words are topics. The CLI adds `project:`
  from the git repository it runs in (`--project` > `PARCEL_PROJECT` > the
  origin's name, else the main checkout's folder) as an *auto tag*: sent as
  `auto_tags`, applied only when the call creates the parcel, never on a new
  version, so a tag someone removed doesn't come back. There is no "auto"
  marker in the UI; the version's source already says it came from the CLI.
- **Writes.** Create: `tags` (explicit) and `auto_tags`. `PATCH`: `tags`
  replaces the set, or `add_tags`/`remove_tags` apply `(current ∪ add) −
  remove` under the row lock (a tag in both, or `tags` with either, is a
  400). A new version takes `add_tags` only. Tags, slug and archive changes
  never bump `updated_at` (organizing isn't editing, and it would reorder
  lists); title, description and visibility do.
- **Vocabulary.** `GET /tags` counts tags over the parcels the same filters
  select (so the counts are facet counts), most used first. `POST
  /tags/rename` (into an existing tag the two merge) and `POST /tags/delete`
  act on the caller's own parcels in one `UPDATE` under the owner lock; the
  delete is a body, not a path, because tags can hold `/`.
- **Filters.** Repeated `tag=` means all of them (AND). The SPA lets one
  `project:` and one `kind:` be selected; the API doesn't care.

### Archive

`archived_at` set by `PATCH archived: true`. Lists, tag counts and search take
`archived=exclude|include|only` (default `exclude`); an archived parcel stays
viewable by URL, with a badge. A new version un-archives (the trigger above:
publishing means it's live again, and a slug upsert would otherwise hide a
fresh plan). The legacy `/shared` endpoint lists archived parcels as before.

### Listing and search

- `GET /parcels` is one builder for every scope: `mine`, `shared` (shared
  with me by email, today's `/shared` semantics) and `all`; the same scoped
  relation feeds results, totals, snippets and tag counts. Internal and
  public parcels nobody shared with you are still not listed. Items are
  `ParcelListItem`: the summary's fields flattened (so older clients keep
  working) plus `match` and, for someone else's parcel, `shared`.
  `ParcelSummary.url` stays `<app>/p/<id>` for good (an old CLI appends
  `/v/<n>` to it); `canonical_url` is the clean URL when there is a slug.
- **Query semantics.** In short: every word must match; when nothing does,
  parcels matching any of the words are returned, best first, and marked
  relaxed; operators (quotes, `-word`, `or`) turn that off. Exactly: `q`
  matches by substring (ILIKE) on id, title, description and slug, **or**
  `search_vec @@ websearch_to_tsquery('english', q)` (every word, stemmed,
  in the title, description, tags or the entry page's text; quotes, `or`
  and `-word` work as in a web search). Order with `q`: `ts_rank_cd` (title
  over description and tags over page text), then `updated_at`, then id;
  without, `updated_at`. A query that parses to nothing (stop words,
  punctuation) falls back to the substring match alone.
- **Relaxed fallback (plan 002).** When that exact clause matches nothing
  in the caller's scope and filters (one `EXISTS` probe,
  `db::parcels::search_mode`) and the query is *relaxable* — at least two
  whitespace-separated words, no `"`, no word starting with `-`, no word
  `or` (Rust, `parcels::relaxable`), and at least two distinct lexemes
  (`cardinality(tsvector_to_array(to_tsvector('english', q)))`, so
  `plan-002` or `e-mail` alone never relaxes) — the list reruns with the
  full-text branch's query replaced by an OR of the query's lexemes, built
  in SQL with hand-written quoting (`quote_literal` would write `E'…'`,
  which the tsquery parser rejects); the substring branches stay. Relaxed
  order: how many of the lexemes a parcel has, then `ts_rank_cd` over the OR
  query, then `updated_at`, id (plain `ts_rank_cd` over an OR would put a
  one-word title match above a page with all but one word); snippets mark
  the OR terms. `ParcelList.relaxed` is `true` then (absent otherwise, so
  exact bodies are unchanged, and old clients ignore it). `GET /tags` takes
  `mode=auto|exact|relaxed`: `auto` (default) runs the same probe (`prefix`
  plays no part); the SPA passes the list's mode to every facet request,
  the main one and each swap, so the sidebar counts what the list shows;
  `TagList.relaxed` says which ran. The probe, page and total are separate
  statements: under concurrent writes they agree on a best-effort basis.
  The probe of a relaxable query that matches nothing scans the caller's
  scope once more (measured in plan 002's artifacts). Feature
  `search-relaxed`.
- **The index.** `search_vec` is maintained by triggers (never by the app):
  title (weight A), description and tags (B) and page text (D), all with the
  `english` configuration the query uses. `parcels_a_version_changed` runs
  before `parcels_search_b_vec` (Postgres orders triggers by name) so a
  version change clears the text before the vector is built. Comment and
  data revision bumps set none of the watched columns and skip both. Since
  0009 the vector is rebuilt only when one of its four inputs changed (a
  visibility PATCH, or a version bump with no stored text, keeps it), so a
  change to `parcel_search_document` or the text-search configuration must
  recompute existing rows explicitly, as 0006 did.
- **Page text** is the current version's entry page only, ≤ 128 KiB, from a
  hand-written stripper (HTML: no script/style/template/noscript/svg, tags
  and comments dropped, entities decoded, breaks at blocks; Markdown: the
  text and code events). It is extracted in a spawned task after the publish
  commits (publish latency is unchanged; no I/O inside the transaction),
  written guarded by `current_version` so a slow extraction can't store an
  old version's text. A loop (own cadence, not the GC's) sweeps up whatever is
  missing every 60 s in batches of 50 within a 20 s budget: parcels from before
  search, versions an older revision published, transient failures (five
  tries per version, `search_attempts`). `parcel-server reindex` drains
  everything with no cap; run it once after the deploy that adds search.
  Failures log value-free (invariant 5).
- **Snippets** are `match: {segments: [{text, hit}]}`, built from
  `ts_headline` with `\u{2}`/`\u{3}` markers (extraction strips them) only
  for the page that is returned, and only when a term was marked. Page text
  is user content shown on the app origin: every consumer renders segments
  as text (React text nodes; the CLI through `printable()`), never HTML, and
  biome's `noDangerouslySetInnerHtml` is an error.

### Access and privacy

- **Owner handle (decision).** Anonymous callers never receive
  `owner_handle` for a parcel without a slug (they get no owner name either).
  A slugged public parcel's `canonical_url` holds the handle by construction:
  setting a slug is the owner opting in to a readable link. A 409 from
  `PATCH /me` says only that a handle is in use.
- A private parcel's title, tags, slug, page text and source never reach
  another user's lists, counts, snippets, tag vocabularies or by-slug (a
  sentinel test pins this); by-slug obeys invariant 3.
- A version's `source` is the owner's only and never holds credentials.

### HTTP additions

Every new route is in the HTTP surface table below; request and response
types are in `crates/parcel-types/src/api.rs`: `GET /tags`, `POST
/tags/rename`, `POST /tags/delete`, `GET /parcels/by-slug`, `PATCH /me`,
the `scope`/`tag`/`archived`/`q` parameters of `GET /parcels`, `path` on
`/parcels/{id}/view`, `slug`/`tags`/`auto_tags`/`source` in create and
version metadata, `tags`/`add_tags`/`remove_tags`/`slug`/`archived` in
`PATCH`, and `features` on `/info`. Only request metadata types deny unknown
fields; responses grow additively (new fields are optional or default), so
an old client parses a new server.

## Handles, slugs and sources (v0.5)

Clean URLs `/@<handle>/<slug>` next to `/p/<id>` (which works forever), a
stable name a republish can target, and where each version came from.

- **Handles** live in `user_handles (handle PK, user_id, is_current)`: one
  key over current and retired handles, so a handle once used is that
  user's for good. Allocation is one SQL function,
  `parcel_allocate_handle(user, email)` (migration 0007), used by the
  backfill, sign-in and lazily for a user an older revision created: the
  email's local part (lowercased, `+suffix` dropped, `.`/`_` → `-`, other
  characters dropped, ≤ 32), then `-2`, `-3`, … Renaming (`PATCH /me`) makes
  the new one current; the old one keeps resolving.
- **Slugs** (`parcels.slug`, unique per owner): 1–4 `/`-separated segments
  of `[a-z0-9]+(-[a-z0-9]+)*`, ≤ 100 characters; no segment shaped like a
  parcel id and no `p` or `v` segment, so no clean URL reads as `/p/<id>` or
  a `/v/<n>` pin (clean URLs pin with `?v=<n>`; an id URL pins only with `/v/<n>`, its `?v=` is ignored). Renaming or clearing one
  leaves a redirect row (`parcel_slug_redirects`) until another of the
  owner's parcels takes the name.
- `ParcelSummary.url` stays `<app>/p/<id>`; `canonical_url` is the clean URL
  when there is a slug, else `url`. Clients show and copy `canonical_url`.
- **Owner handle privacy** is decided above, in "Tags, slugs, archive and
  search (v0.5)", "Access and privacy".
- **by-slug** resolves handle (current or retired) → user, then slug or
  redirect → parcel, then `readable()`. Every failure — unknown handle or
  slug, no access, public links off, malformed input — is one constant 404
  body with identical headers (invariant 3).
- **Publish upsert.** `CreateParcelMetadata.slug`: when the caller has a
  parcel with that slug, or a redirect row for it, the publish versions that
  parcel (files replaced; only the fields sent change, the title is never
  re-derived; `tags` added, `auto_tags` ignored; `base_version` checked;
  un-archives), decided under the owner's advisory lock so two first
  publishes of one slug make one parcel. Otherwise it creates the parcel
  with that slug (`base_version` → 409, since nothing has the slug yet).
- **Sources.** `parcel_versions.source` = `{repo, branch, commit, host,
  client}`, each ≤ 200 printable characters; `repo` reduced to
  `host[:port]/path` (userinfo, query, fragment and `.git` dropped; local
  paths omitted; anything else refused with a 400 that names only the field,
  never the value). Shown to the owner only, wherever versions are returned;
  a restore copies the restored version's source; MCP publishes record
  `{client: "mcp"}`.

## Pages within a parcel (v0.5)

A version may hold several pages, linked to each other with relative links
(they resolve under the same capability prefix, so following one in the
frame just works).

- A **page** is any `.html`/`.htm`/`.md`/`.markdown` file of a version.
  `version_files.page_title` (migration 0008) keeps its `<title>` or first
  `# heading`, read at publish: plain text, control characters stripped,
  ≤ 200 characters; carried with the file when a merge keeps it, copied by a
  restore, `NULL` for a page without one and for rows from before 0008
  (clients then show the path). Titles are shown as text, never markup.
- `FileEntry.page_title?`; `ParcelDetail.pages` and `VersionDetail.pages` =
  `[{path, title|null}]`, the entry first, then by path (byte order). Anyone
  who sees the file list sees them.
- `GET /parcels/{id}/view?version=&path=` mints the grant for that page;
  a `path` that isn't a page of that version is a 404. The grant still
  covers the whole version.
- **Markdown pages.** The entry, when Markdown, is always rendered into the
  styled shell. Any other Markdown file is rendered only when it is opened
  as a page: `Sec-Fetch-Dest: document` or `iframe`, or, with no
  `Sec-Fetch-Dest`, an `Accept` naming `text/html`; anything else (a page's
  `fetch()`) gets the raw text. The header picks a representation, never
  access: both need the same capability and carry the same sandbox headers.
  Both variants (304s included) carry `Vary: Sec-Fetch-Dest, Accept` and distinct
  ETags; the variant is decided before the `If-None-Match` check. A rendered
  page gets the helper like any HTML page. Raw HTML inside Markdown passes
  through, so a rendered `.md` page is equivalent to an uploaded HTML page
  and sandboxed the same way.
- Search indexes the entry page only.

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
rendered Markdown pages) gets `<script src="/_parcel/helper.<hash>.js">`
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
| page → viewer | `ready` | `path, title, version, token, hash?` (on load and on `hello`; `token` is the page's own capability token, read from its URL; `hash` is the page's `#fragment` when it has one) |
| page → viewer | `anchor` | `anchor` (a click in comment mode) |
| page → viewer | `pin-click` | `number` |
| page → viewer | `located` | `found: number[], missing: number[]` |
| page → viewer | `escape` | — (Escape pressed in comment mode) |
| page → viewer | `hash` | `hash` (on `hashchange`: the page's `#fragment`, `""` when it has none) |
| page → viewer | `bye` | — (`pagehide`: the viewer stops sending until the next verified `ready`) |

Viewer messages carry `source: 'parcel-viewer'`, page messages
`source: 'parcel-helper'`. A reported `hash` is at most 200 printable ASCII
characters including the `#` (a longer or odd one reads as none). The viewer accepts page messages only from its
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

Not now (tracked as GitHub issues): groups from provider claims (#14), an editor
role (#13), invite emails/notifications (#15), an admin UI and SCIM (#17),
discovery of internal parcels (#16).

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
revision (now kept, nullable, then dropped by migration 0005 once no running revision read it); and signature forgery wasn't tested. Not
verified live: Okta (no account here; its setup is in `deploy/README.md`).

### v0.5 (tags, slugs, archive, search, pages)

| Check | Result |
|---|---|
| `cargo test --workspace` (incl. the second `DATABASE_URL` pass) | 379 passed |
| `pytest tests/functional` (incl. the released 0.1.2 CLI against the new server) | 51 passed |
| `bun test` / Playwright e2e on the production build | 231 / 69 passed |
| clippy (both feature sets), `cargo deny`, crypto guard, fmt, Spectral, biome/eslint, tsc | clean |
| Browser walkthrough (playwright-cli, 46 screenshots) | facets with filter-scoped counts, token search and autocomplete, full-text snippets, Mine/Shared/All, archive and unarchive, Edit details (tags, slug conflict), the Tags page (rename, merge, delete), handle change and the old-handle redirect, a two-page parcel (Pages menu, in-page links, deep link with fragment, a comment pinned on page 2), phone width and dark theme: works |

Every milestone commit ran the full gate and a Codex review (gpt-6-astra for
the subtle ones, gpt-6.1-sol for the rest); findings are in the commit
messages. Two things the plan got wrong and the build corrected: the vector
trigger must list `current_version` (an `UPDATE OF` trigger doesn't see
columns another trigger sets), and tags are indexed with the `english`
configuration the query uses.

Deviations from the original brief, by design:
- CLI auth is a device flow + app-local `pcl_` tokens rather than slauth PATs or
  a loopback OAuth client (see "Auth"). Accepting slauth PATs too is tracked in #22.
- Nothing is deployed: Cloud Run/Neon/GCS/DNS setup is scripted and documented in
  `deploy/README.md` but not executed; GitHub workflows are written but not run.
- slauth registration lives on slauth's local `feature/add-parcel-app` branch
  (committed, not pushed), which is currently checked out in `../slauth`.

# CLAUDE.md — parcel conventions

Read this and [PLAN.md](PLAN.md) (architecture, assumptions, HTTP surface)
before changing anything.

## What this is

Parcel publishes HTML pages (plus supporting files) to private, sandboxed
URLs — a self-hosted take on Claude's artifacts. One Rust server answers two
origins: the **app** (SPA + `/api/v1`) and the **content origin** (user files
behind HMAC capability URLs, always CSP-sandboxed). People sign in through
one or more **OIDC providers** (slauth, Okta, Keycloak…; the server is the
OIDC client and issues a signed session cookie); the `parcel` CLI signs in
with a device flow and `pcl_` API tokens. Parcels are private, shared with
people by email, internal (everyone signed in) or public. A bundled skill
(`skills/parcel`) teaches agents to use it, and an **MCP server** at
`<app>/mcp` (`src/mcp`, on stridelabs-mcp) gives connected assistants
(Claude, Claude Code) the same operations as tools.

## Layout

| Path | What |
|---|---|
| `crates/parcel-types` | Wire types + validation rules shared by server, CLI and tests (paths, media types, ids). Feature `openapi` adds utoipa derives. |
| `crates/parcel-server` | axum + sqlx service (lib + thin `main.rs`). `migrations/`, committed `openapi.json`, `tests/` (real Postgres). |
| `crates/parcel-cli` | The `parcel` binary (blocking reqwest). Embeds `skills/parcel`. |
| `frontend/` | React 19 + Vite + Tailwind v4 + zustand SPA (bun). |
| `skills/parcel/` | The CLI agent skill (`SKILL.md` + `references/`), embedded in the CLI and the `parcel` Claude Code plugin (`.claude-plugin/`, the repo's marketplace). |
| `plugins/parcel-mcp/` | The `parcel-mcp` Claude Code plugin: the MCP server (`.mcp.json`, OAuth client `parcel-claude-code`, callback port 3118) and its skill; `references/authoring.md` is a tested copy of the CLI skill's. |
| `tests/functional/` | pytest suite driving the real binaries over HTTP (GCS emulator + test JWKS). |
| `deploy/`, `Dockerfile`, `.github/workflows/` | Cloud Run + Neon + GCS (production live at parcel.stridelabs.ai). CI, release and deploy run on GitHub ([charliek/parcel](https://github.com/charliek/parcel), public); see RELEASING.md and deploy/README.md. |

## Toolchain and local stack

- Rust 1.97.1 via `.mise.toml` / `rust-toolchain.toml`: run `mise exec -- cargo …`.
- Move `Cargo.lock` with targeted `cargo update -p <crate>` (a manifest bump
  re-resolves only what it touches); don't delete and regenerate it, which
  moves every dependency at once. (It was seeded from slauth's lock while
  utoipa-swagger-ui 9 needed the yanked `zip 3.0.0`; utoipa-swagger-ui 10
  takes zip 8, so nothing in it depends on a yanked release any more.)
- One crypto provider, aws-lc-rs: reqwest `rustls`, sqlx
  `tls-rustls-aws-lc-rs`, jsonwebtoken `aws_lc_rs`. Never enable a `ring`
  flavour or jsonwebtoken `rust_crypto` (two providers or backends panic at
  the first TLS connect or JWT sign/verify).
  `cargo tree -i ring --workspace --all-features -e all --target all` prints
  nothing, and the lock has no `rsa`.
- `docker compose up -d` — Postgres on **5439**, fake-gcs-server on **4447**.
- `prox up -d` — backend :8106 and Vite :5178 behind
  `https://parcel.local.stridelabs.ai`, content on
  `https://parcelcontent.local.stridelabs.ai`, backend direct on
  `https://parcelapi.local.stridelabs.ai`. slauth must be up too
  (`cd ../slauth && prox up -d`; its registry has the `parcel` web client and
  the `parcel-claude` / `parcel-claude-code` MCP connectors — after changing
  `hydra/clients.json` locally, run its `scripts/seed-hydra-clients.sh`).
  Local test login: `../slauth/test-users.md`.
- Optional second provider: `docker compose --profile keycloak up -d keycloak`
  (see `dev/keycloak/README.md` for its users and the `.env` lines).
- `AUTH_MODE=dev` (in `.env`) bypasses slauth with a fixed user — for e2e and
  offline work only; refused when `APP_ENV=production`.

## Quality gate (before every commit)

```bash
mise exec -- cargo fmt --all
mise exec -- make lint          # clippy -D warnings, default + production features
mise exec -- make test          # REQUIRES docker compose up -d; fails loudly, never skips
make spectral                   # OpenAPI lint
make functional                 # pytest against the real binaries
make frontend-check             # biome/eslint, tsc, bun test, vite build
scripts/e2e-stack.sh &          # prod-shaped server (built SPA, dev auth) on :18110
PARCEL_E2E_BASE_URL=http://localhost:18110 make e2e   # Playwright
```

Driving the real UI with `playwright-cli`: `.playwright/cli.config.json`
(git-ignored) sets `ignoreHTTPSErrors` because Playwright's Chromium doesn't
trust prox's local CA.

## Load-bearing invariants

1. **User content never runs on the app origin.** Content is served only on
   the content host, always with the sandbox CSP (no `allow-same-origin`),
   `Referrer-Policy: no-referrer`, and `frame-ancestors <app>`. The raw-file
   API on the app origin is `Content-Disposition: attachment` + sandbox CSP.
   The viewer iframe repeats the sandbox. Don't weaken any of these.
2. **Host dispatch happens before routing** (`http::server::HostDispatch`): no
   API route on the content host, no file on the app host.
3. **Parcels are 404 to anyone without access** (never 403), so ids can't be
   probed. Access is decided in one place, `http::views::resolve_access`
   (the table in PLAN.md, "Sign-in providers and sharing"); content grants
   re-check it on every request (`content::member_still_allowed`).
4. **Blob writes precede the rows that reference them**, and GC only collects
   blobs that are unreferenced *and* untouched for the grace period. Don't
   reorder `publish.rs`'s steps.
5. **Never log secrets** — tokens, session cookies, provider client secrets
   (`config::Secret`), ID tokens, content capability URLs (the trace span
   redacts `/v/<token>` and `/d/<token>`), signing keys.
6. **OpenAPI is pinned**: every `/api/v1` route goes through
   `OpenApiRouter::routes(routes!(..))` and appears in
   `tests/openapi_shape.rs`; regenerate `openapi.json` with `make openapi`.
7. **Env var names are an operator contract** (see `.env.example`,
   `config.rs`, `deploy/cloud-run-service.yaml`).
8. **Page data is written only with owner authority**: the owner's API
   credentials, or an owner-scope content capability (minted only when the
   owner views their own parcel). Public- and member-scope capabilities
   read, never write, and stop working when access is withdrawn.
9. **The page helper talks only to the app origin** (baked in at startup)
   and the viewer accepts its messages only from its own iframe, as
   untrusted input. Nothing sensitive crosses: anchors and comment numbers,
   never comment text, authors or tokens. Anonymous viewers never see
   comments. Protocol: PLAN.md, "Comments, live updates and page data".
10. **Sign-in state lives in signed cookies, not tables.** The session is
   `HMAC(user, epoch, expiry, provider)`; the server stores no provider
   tokens. Cookie-authenticated writes need `Origin` = the app origin, and a
   cookie only counts for `Sec-Fetch-Site` same-origin/none. On `/api/v1` the
   only bearer credential is a `pcl_` token. `/mcp` also takes the MCP
   provider's access tokens minted for its resource (`aud` = `<app>/mcp`),
   which work nowhere else, and strips them before any tool runs. Every
   sign-in needs a provider-verified email (or `TRUST_EMAIL`) in that
   provider's allowed domains, because shares and account linking go by
   email; an MCP token's first use is a sign-in under the same rules.

## Conventions

- Typed errors (`thiserror`) in libraries; `stridelabs_http::AppError` at the
  HTTP boundary, with `anyhow` only through `AppError::Internal`.
- Config reads go through `Config::from_lookup` (injectable); tests never
  mutate process env. Rust integration tests get an isolated Postgres schema
  per `TestApp` (`tests/common/mod.rs`).
- Match the surrounding comment density; comments explain *why*.
- Commit straight to `main` while the project is being set up.

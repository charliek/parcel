# CLAUDE.md — parcel conventions

Read this and [PLAN.md](PLAN.md) (architecture, HTTP surface) before changing
anything; releases and deploys: RELEASING.md, deploy/README.md.

## What this is

Parcel publishes HTML pages (plus files) to private, sandboxed URLs. One Rust
server answers the **app** origin (SPA, `/api/v1`, `/mcp`) and the **content
origin** (user files behind HMAC capability URLs, always CSP-sandboxed).
Sign-in is server-side OIDC (slauth, Okta, Keycloak…); the CLI uses a device
flow and `pcl_` tokens. Parcels have tags, an optional slug, searchable text.

Layout: `crates/parcel-{types,server,cli}` (server: `migrations/`, committed
`openapi.json`, `tests/` on real Postgres; the CLI embeds `skills/parcel`),
`frontend/` (React 19, Vite, bun), `plugins/parcel-mcp/` (OAuth client
`parcel-claude-code`, callback port 3118; its `authoring.md` must equal the CLI
skill's), `tests/functional/` (pytest), `deploy/` (Cloud Run + Neon + GCS,
live at parcel.stridelabs.ai; CI/release/deploy on GitHub, charliek/parcel).

## Toolchain and local stack

- Rust 1.97.1: `mise exec -- cargo …`; `Cargo.lock` moves only by `cargo update -p`.
- One crypto provider, aws-lc-rs: never enable a `ring` flavour or jsonwebtoken
  `rust_crypto` (two providers panic at the first TLS connect or JWT operation);
  `scripts/crypto-guard.sh` fails on `ring` or `rsa`.
- `docker compose up -d` (Postgres :5439, fake-gcs :4447); `prox up -d`
  (`https://parcel.local.stridelabs.ai`, content `parcelcontent.local…`,
  backend direct `parcelapi.local…`) with slauth up (`../slauth`, logins in its
  `test-users.md`; after changing its `hydra/clients.json`, run its
  `scripts/seed-hydra-clients.sh`). Second provider: `docker compose --profile
  keycloak up -d keycloak` (`dev/keycloak/README.md`). `AUTH_MODE=dev` (fixed
  user): e2e and offline only, refused in production.
  `playwright-cli` needs a git-ignored `.playwright/cli.config.json`
  (`ignoreHTTPSErrors`).

## Quality gate (before every commit)

```bash
mise exec -- cargo fmt --all
mise exec -- make lint          # clippy -D warnings (both feature sets), cargo deny, crypto guard
mise exec -- make test          # REQUIRES docker compose up -d; fails loudly, never skips; 2nd pass with DATABASE_URL set
make spectral                   # OpenAPI lint
make functional                 # pytest against the real binaries
make frontend-check             # biome/eslint, tsc, bun test, vite build
scripts/e2e-stack.sh &          # prod-shaped server (built SPA, dev auth) on :18110
PARCEL_E2E_BASE_URL=http://localhost:18110 make e2e   # Playwright
```

## Load-bearing invariants

1. **User content never runs on the app origin.** Content is served only on
   the content host, always with the sandbox CSP (no `allow-same-origin`),
   `Referrer-Policy: no-referrer`, and `frame-ancestors <app>`. The app-origin
   raw-file API is `Content-Disposition: attachment` + sandbox CSP; the viewer
   iframe repeats the sandbox. Don't weaken any of these.
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
   owner views their own parcel). Public- and member-scope capabilities read,
   never write, and stop working when access is withdrawn.
9. **The page helper talks only to the app origin** (baked in at startup)
   and the viewer accepts its messages only from its own iframe, as
   untrusted input. Only anchors and comment numbers cross, never comment
   text, authors or tokens; anonymous viewers never see comments. Protocol:
   PLAN.md, "Comments, live updates and page data".
10. **Sign-in state lives in signed cookies, not tables.** The session is
   `HMAC(user, epoch, expiry, provider)`; the server stores no provider
   tokens. Cookie writes need `Origin` = the app origin, and a cookie only
   counts for `Sec-Fetch-Site` same-origin/none. On `/api/v1` the only bearer
   credential is a `pcl_` token; `/mcp` also takes the MCP provider's access
   tokens minted for its resource (`aud` = `<app>/mcp`), which work nowhere
   else and are stripped before any tool runs. Every sign-in (an MCP token's
   first use too) needs a provider-verified email (or `TRUST_EMAIL`) in that
   provider's allowed domains, because shares and account linking go by email.

Also (PLAN.md, "Tags, slugs, archive and search"):

- Snippets and page text are user content: render as text (React text nodes,
  the CLI's `printable()`), never HTML. A version's `source` is owner-only and
  never holds credentials.
- Refs stay unambiguous: no slug segment is id-shaped, `p` or `v`.
- Triggers own `search_vec` and the clearing of `search_text` and
  `archived_at` (migration 0006); migrations stay additive (two revisions run
  together in a Cloud Run rollover).

## Conventions

- Typed errors (`thiserror`) in libraries; `stridelabs_http::AppError` at the
  HTTP boundary, `anyhow` only via `AppError::Internal`.
- Config reads go through `Config::from_lookup`; tests never mutate process
  env and get an isolated Postgres schema per `TestApp` (`tests/common/mod.rs`).
- Comments explain *why*, at the surrounding density.
- Commit straight to `main` while the project is being set up (larger plans
  use a feature branch and one PR).

## Shared libraries (stridelabs-*)

The owner's guidance, shared by every app on the libraries. parcel pins
`stridelabs-rust` at one git tag (`crates/parcel-server/Cargo.toml`): config,
observability, http, auth, mcp, postgres, plus testing and `test-support`
in dev. Keep them strong but pragmatic: a library never holds parcel back.

- **Where code goes.** Only parcel needs it: plain code, or a module built to
  lift out (narrow API, no parcel config, database or types, own tests). Other
  apps already do it: extend the library, opt-in and additive. Else: parcel code.
- **What earns a place.** A convention (config, logging, CI guards, error
  shapes, auth) or a capability two or more apps reuse. Shipping apps'
  evidence (slauth, imgmcp, tapper, parcel) outweighs experiments' (lumen,
  spendwise-rs, limen). Auth stays deployment-flexible. Curate: prefer
  deleting to adding machinery.
- **Fix upstream** under the library's CLAUDE.md, CI and review (CHANGELOG
  line, additive); pin the merged `rev` until a tag has it. If that blocks a
  milestone, implement here first and upstream in parallel.
- **Layout, pins, releases.** `crates/<name>` as `stridelabs-<name>`, lockstep
  versions, consumers pin by `tag`, all together. Release: ping every consumer
  and check nobody claimed the next minor; docs-only release commit (dated
  CHANGELOG, README `tag` snippets); local merge commit, annotated tag on it;
  send peers the SHA.
- **Gap.** parcel isn't in the library's `scripts/downstream-check.sh` yet:
  add it at the next tag (Postgres 5439, fake-gcs 4447, env unset).

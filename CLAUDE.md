# CLAUDE.md — parcel conventions

Read this and [PLAN.md](PLAN.md) (architecture, assumptions, HTTP surface)
before changing anything.

## What this is

Parcel publishes HTML pages (plus supporting files) to private, sandboxed
URLs — a self-hosted take on Claude's artifacts. One Rust server answers two
origins: the **app** (SPA + `/api/v1`) and the **content origin** (user files
behind HMAC capability URLs, always CSP-sandboxed). Users sign in through
**slauth** (OAuth2 + PKCE); the `parcel` CLI signs in with a device flow and
`pcl_` API tokens. A bundled skill (`skills/parcel`) teaches agents to use it.

## Layout

| Path | What |
|---|---|
| `crates/parcel-types` | Wire types + validation rules shared by server, CLI and tests (paths, media types, ids). Feature `openapi` adds utoipa derives. |
| `crates/parcel-server` | axum + sqlx service (lib + thin `main.rs`). `migrations/`, committed `openapi.json`, `tests/` (real Postgres). |
| `crates/parcel-cli` | The `parcel` binary (blocking reqwest). Embeds `skills/parcel`. |
| `frontend/` | React 19 + Vite + Tailwind v4 + zustand SPA (bun). |
| `skills/parcel/` | The agent skill (`SKILL.md` + `references/`), also a Claude plugin via `.claude-plugin/`. |
| `tests/functional/` | pytest suite driving the real binaries over HTTP (GCS emulator + test JWKS). |
| `deploy/`, `Dockerfile`, `.github/workflows/` | Cloud Run + Neon + GCS. Prepared, not yet run (no GitHub repo yet). |

## Toolchain and local stack

- Rust 1.97.1 via `.mise.toml` / `rust-toolchain.toml`: run `mise exec -- cargo …`.
- `Cargo.lock` was seeded from slauth's lock on purpose: utoipa-swagger-ui 9
  needs the **yanked** `zip 3.0.0`, which only resolves from an existing lock.
  Never delete/regenerate the lock from scratch.
- `docker compose up -d` — Postgres on **5439**, fake-gcs-server on **4447**.
- `prox up -d` — backend :8106 and Vite :5178 behind
  `https://parcel.local.stridelabs.ai`, content on
  `https://parcelcontent.local.stridelabs.ai`, backend direct on
  `https://parcelapi.local.stridelabs.ai`. slauth must be up too
  (`cd ../slauth && prox up -d`; its `feature/add-parcel-app` branch registers
  the `parcel` client). Local test login: `../slauth/test-users.md`.
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
```

## Load-bearing invariants

1. **User content never runs on the app origin.** Content is served only on
   the content host, always with the sandbox CSP (no `allow-same-origin`),
   `Referrer-Policy: no-referrer`, and `frame-ancestors <app>`. The raw-file
   API on the app origin is `Content-Disposition: attachment` + sandbox CSP.
   The viewer iframe repeats the sandbox. Don't weaken any of these.
2. **Host dispatch happens before routing** (`http::server::HostDispatch`): no
   API route on the content host, no file on the app host.
3. **Private parcels are 404 to non-owners** (never 403), so ids can't be probed.
4. **Blob writes precede the rows that reference them**, and GC only collects
   blobs that are unreferenced *and* untouched for the grace period. Don't
   reorder `publish.rs`'s steps.
5. **Never log secrets** — tokens, content capability URLs (the trace span
   redacts `/v/<token>`), signing keys.
6. **OpenAPI is pinned**: every `/api/v1` route goes through
   `OpenApiRouter::routes(routes!(..))` and appears in
   `tests/openapi_shape.rs`; regenerate `openapi.json` with `make openapi`.
7. **Env var names are an operator contract** (see `.env.example`,
   `config.rs`, `deploy/cloud-run-service.yaml`).

## Conventions

- Typed errors (`thiserror`) in libraries; `stridelabs_http::AppError` at the
  HTTP boundary, with `anyhow` only through `AppError::Internal`.
- Config reads go through `Config::from_lookup` (injectable); tests never
  mutate process env. Rust integration tests get an isolated Postgres schema
  per `TestApp` (`tests/common/mod.rs`).
- Match the surrounding comment density; comments explain *why*.
- Commit straight to `main` while the project is being set up.

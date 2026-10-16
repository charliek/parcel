# Parcel web app

The React SPA for Parcel: the "My parcels" dashboard, the sandboxed viewer
(`/p/:id`), uploads (`/new`), API tokens, CLI setup and CLI device-login
approval. Built with React 19, Vite 7, Tailwind CSS v4, zustand, React Query,
react-router v7 and Radix UI, the same stack as slauth.

## Commands

```bash
bun install          # dependencies
bun run dev          # Vite on :5178; proxies /api, /openapi.json, /docs to :8106
bun run build        # typecheck + production build into dist/
bun run lint         # biome check + eslint (must be clean)
bun run typecheck    # tsc -b (app, unit tests, configs and e2e)
bun test             # unit tests (src/**/*.test.ts)
bun run e2e          # Playwright end-to-end tests (see below)
```

`PARCEL_BACKEND_URL=http://host:port bun run dev` points the dev proxy at a
different backend. Locally, prox serves the app at
`https://parcel.local.stridelabs.ai`, which forwards to Vite on :5178.

In production the Rust backend serves `dist/` (`PARCEL_STATIC_DIR`) with an
`index.html` fallback for client routes. Asset URLs are absolute (`/assets/…`).

## Configuration happens at runtime

Nothing is baked in at build time. On boot the app fetches `GET /api/v1/info`
and uses it for:

- **Auth mode.** In `slauth` mode it runs OAuth2 Authorization Code + PKCE
  against `auth.issuer`, with `auth.client_id` and `audience=auth.audience`.
  In `dev` mode the backend treats every request as the dev user, so the app
  never starts OAuth and hides "Sign out".
- **Upload limits and allowed extensions**, which `/new` checks before upload.

## Auth behavior

- Access and refresh tokens live in memory only. After a reload the app
  re-authenticates with one silent `prompt=none` attempt per browser session.
  Token refreshes are deduped, and signing out in one tab signs out every tab.
- Sign-in is optional where it can be. The viewer first requests
  `/api/v1/parcels/{id}/view` with whatever token it already has (often none),
  so public parcels render for anonymous visitors. It falls back to silent
  re-auth and then a "Sign in to view" screen only after a 401/403/404.
- `/new`, `/tokens`, `/cli` and `/device` need an account. On these pages the
  app calls `/api/v1/me`, tries silent re-auth once if that returns 401, then
  shows a sign-in screen. The return path, including `?code=` on `/device`, is
  kept through the login redirect.
- `/` shows the dashboard when signed in and the landing page otherwise. It
  only tries silent re-auth if this browser has signed in before, so
  first-time visitors don't bounce through slauth.
- Browser storage keys all start with `parcel_`, and every storage access is
  wrapped in try/catch.

The pure decision logic is in `src/lib/auth/policy.ts` and is unit-tested.

## End-to-end tests

The specs in `e2e/` expect a parcel-server running in dev auth mode
(`AUTH_MODE=dev`). They cover the dashboard and search, creating a parcel by
file and folder upload, rendering in the sandboxed iframe, public/private,
versions and restore, editing, deletion, tokens, and device-login
approve/deny.

```bash
bunx playwright install chromium   # once
bun run e2e                        # starts or reuses Vite on :5178 (backend on :8106)
PARCEL_E2E_BASE_URL=https://parcel.local.stridelabs.ai bun run e2e   # an already-running stack
```

The backend's `PARCEL_APP_ORIGIN` must match the origin the tests load (for
example `http://localhost:5178`). Otherwise the content host's
`frame-ancestors` policy blocks the viewer iframe.

## Layout

```
src/
  lib/          api client, OAuth/PKCE, token store, types (mirror of parcel-types),
                formatting, upload-path rules, auth policy (+ *.test.ts)
  stores/       zustand: server info, session, theme
  hooks/        React Query hooks, session and silent re-auth hooks
  components/   ui/ (shadcn-style primitives), layout/, viewer/, upload/, tokens/, auth/
  pages/        one component per route
e2e/            Playwright specs
```

# Parcel web app

The React SPA for Parcel: the "My parcels" dashboard, the sandboxed viewer
(`/p/:id`), uploads (`/new`), API tokens, CLI setup and CLI device-login
approval. Built with React 19, Vite 7, Tailwind CSS v4, zustand, React Query,
react-router v7 and Radix UI, the same stack as slauth.

## Commands

```bash
bun install          # dependencies
bun run dev          # Vite on :5178; proxies /api, /auth, /openapi.json, /docs to :8106
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

- **Auth mode and providers.** In `oidc` mode `auth.providers` lists the
  sign-in providers (`{id, label}`) to offer. In `dev` mode the backend treats
  every request as the dev user, so the app shows no sign-in or sign-out.
- **`allow_public`.** When false, "Anyone with the link" is disabled (with the
  reason) wherever visibility is set.
- **Upload limits and allowed extensions**, which `/new` checks before upload.

## Auth behavior

- Sign-in is server-side. The SPA never runs OAuth and never holds a provider
  token: "Sign in" is a full-page navigation to
  `/auth/login?provider=<id>&return_to=<path>`, the server sets an HttpOnly
  session cookie and lands back on `return_to`. API calls send no
  `Authorization` header; the cookie rides along on same-origin requests.
- With one provider, "Sign in" goes straight to it. With several, it opens
  `/signin`, which has one button per provider (the last one used first,
  remembered in `localStorage`). The server also sends failed sign-ins there
  (`/signin?error=<code>&return_to=<path>`), and the page explains each code.
- Sign-out is `POST /auth/logout` (or `?everywhere=true`, on the Tokens page),
  then the browser follows the returned `redirect`: the provider's logout page,
  or `/`. Other tabs are told through `localStorage` and drop the user too.
- A 401 from the API means the session ended (the server clears the cookie in
  that response). The app forgets the user and refetches what is on screen
  signed out, so a public parcel keeps showing.
- Sign-in is optional where it can be. The viewer requests
  `/api/v1/parcels/{id}/view` with whatever session it has (often none), so
  public parcels render for anonymous visitors, and drives its UI from the
  grant's `access`: `owner`, `commenter` (comment mode and composer), `viewer`
  (comments read-only) or `public` (signed out: no comments, a sign-in offer).
- `/new`, `/tokens`, `/cli` and `/device` need an account. Signed out, they
  show the sign-in options. A browser that has signed in before (and didn't
  sign out) goes straight to a single provider instead, at most once every two
  minutes per tab, so a session cookie that doesn't stick can't loop. The
  return path, including `?code=` on `/device`, is kept.
- Browser storage keys all start with `parcel_`, and every storage access is
  wrapped in try/catch.

The pure decision logic is in `src/lib/sign-in.ts` and `src/lib/access.ts`,
and is unit-tested.

## End-to-end tests

The specs in `e2e/` expect a parcel-server running in dev auth mode
(`AUTH_MODE=dev`). They cover the dashboard (tag facets, the token search
with its keyboard flow, full-text snippets, scopes, archived, phone width),
creating a parcel by file and folder upload, rendering in the sandboxed
iframe, pages within a parcel (the Pages menu, `?page=` and the fragment
following links inside the page, comments on other pages), tags and slugs
(the details dialog, the Tags page, clean `/@handle/slug` URLs and the
handle), visibility, sharing, versions and restore, editing, deletion,
comments, live updates, page data, tokens, and device-login approve/deny. Sign-in, the viewer as a
viewer or commenter, and public links turned off are covered by rewriting
`/api/v1/info`, `/api/v1/me` and view grants with Playwright routing.

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
  lib/          api client, types (mirror of parcel-types), sign-in and access
                decisions, formatting, upload-path rules, tag/slug twins of
                parcel-types, dashboard URL state, viewer links and page
                navigation (+ *.test.ts)
  stores/       zustand: server info, session, theme
  hooks/        React Query hooks, session hooks
  components/   ui/ (shadcn-style primitives), layout/, viewer/, upload/, tokens/, auth/,
                dashboard/ (facets, search box), tags/ (chips, chip input, slug field),
                account/ (handle)
  pages/        one component per route
e2e/            Playwright specs
```

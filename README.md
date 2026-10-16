# Parcel

Publish an HTML page — or a folder of them, with scripts, styles, data, images
and fonts — and get back a private link that renders it in a sandbox. Parcel
is a self-hosted take on "artifacts": made for coding agents (Claude Code,
craze, opencode) as much as for people.

- **Private by default.** Sign in with StrideLabs (slauth); only you can open
  your parcels unless you make one public.
- **Versioned.** Every publish is an immutable version; republishing keeps the
  URL. Restore any old version.
- **Sandboxed.** User HTML runs on a separate content origin with an opaque
  origin (CSP `sandbox`), so a parcel can't touch your cookies, storage or the
  app around it.
- **Live.** Open pages reload themselves when a new version is published.
- **Feedback loop.** Click anywhere on a page in comment mode to pin a
  comment; an agent reads them with `parcel comments list`, publishes a fix
  and resolves them.
- **Pages that remember.** `window.parcel.data` keeps JSON between visits:
  every viewer reads it, the owner (or their agent, with `parcel data set`)
  writes it.
- **Agent-ready.** The `parcel` CLI and its bundled skill let an agent publish
  and read parcels with one command and JSON output.

```bash
parcel login                              # opens the browser, approve the code
parcel publish report.html                # → https://parcel.stridelabs.ai/p/k3m9x2qtw8ra
parcel publish ./site --parcel k3m9x2qtw8ra --label "v2 copy"
parcel cat k3m9x2qtw8ra                   # read it back
parcel comments list k3m9x2qtw8ra         # feedback left in the viewer
parcel data set k3m9x2qtw8ra stats today '{"visits": 42}'
parcel skill install                      # teach Claude Code / craze / opencode
```

## Install the CLI

```bash
brew install charliek/tap/parcel            # macOS / Linux (once released)
sudo apt install parcel                     # via https://apt.stridelabs.ai (once released)
cargo install --locked --path crates/parcel-cli   # from source, today
```

## Run it locally

Requires Docker, [mise](https://mise.jdx.dev), [bun](https://bun.sh),
[uv](https://docs.astral.sh/uv/) and [prox](https://github.com/charliek/prox),
plus a running [slauth](../slauth) with parcel registered.

```bash
docker compose up -d                 # Postgres :5439, GCS emulator :4447
cp .env.example .env                 # defaults work under prox
(cd frontend && bun install)
prox up -d                           # backend :8106 + Vite :5178 on *.local.stridelabs.ai
open https://parcel.local.stridelabs.ai
```

Then point the CLI at it: `parcel login --server https://parcel.local.stridelabs.ai`.

## Test

```bash
mise exec -- make test         # Rust unit + integration (real Postgres, GCS emulator)
make functional                # pytest: real server + CLI binaries over HTTP
make frontend-check            # lint, typecheck, unit tests, build
make e2e                       # Playwright (see frontend/README.md)
```

## Docs

- [PLAN.md](PLAN.md) — architecture, decisions, assumptions, HTTP surface
- [CLAUDE.md](CLAUDE.md) — conventions and invariants for contributors and agents
- [deploy/README.md](deploy/README.md) — Cloud Run + Neon + GCS
- [RELEASING.md](RELEASING.md) — CLI releases (brew + apt)
- [skills/parcel/SKILL.md](skills/parcel/SKILL.md) — the agent skill
- `/docs` on a dev server — Swagger UI for the API (`/openapi.json` everywhere)

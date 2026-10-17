# Parcel

Publish an HTML page — or a folder of them, with scripts, styles, data, images
and fonts — and get back a private link that renders it in a sandbox. Parcel
is a self-hosted take on "artifacts": made for coding agents (Claude Code,
craze, opencode) as much as for people.

- **Private by default, easy to share.** Sign in through slauth, Okta,
  Keycloak or any OIDC provider (several at once, e.g. one per office). Share
  a parcel with people by email as viewer or commenter, open it to everyone
  signed in, or make it public (which a server can turn off).
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
  and read parcels with one command and JSON output, and an MCP server at
  `/mcp` gives Claude (claude.ai) and Claude Code the same as tools; both
  come as Claude Code plugins.

```bash
parcel login                              # opens the browser, approve the code
parcel publish report.html                # → https://parcel.stridelabs.ai/p/k3m9x2qtw8ra
parcel publish ./site --parcel k3m9x2qtw8ra --label "v2 copy"
parcel cat k3m9x2qtw8ra                   # read it back
parcel share k3m9x2qtw8ra ada@acme.com --role commenter
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

## Use it from Claude

Agents have two first-class interfaces with the same operations: the MCP server
(`https://<your parcel>/mcp`; production `https://parcel.stridelabs.ai/mcp`)
and the `parcel` CLI with its skill. Both publish, read, revise, share and
delete parcels, work through the comments people leave in the viewer, and
update the data pages keep. The usual loop: the agent publishes a plan or page,
you comment on it in the browser, then you ask the agent to update it from the
feedback.

**Claude Code plugins** (this repo is their marketplace):

```
/plugin marketplace add charliek/parcel
/plugin install parcel-mcp@parcel      # the MCP server; then /mcp to sign in
/plugin install parcel@parcel          # the CLI skill (needs the parcel CLI)
```

`parcel-mcp` registers the server with Parcel's pre-registered OAuth client
(`parcel-claude-code`, callback port 3118; the provider, slauth, has no dynamic
client registration) and a skill for the publish → comment → revise loop.
Without the plugin:
`claude mcp add --transport http --client-id parcel-claude-code --callback-port 3118 parcel https://parcel.stridelabs.ai/mcp`.

**Claude.ai:** Settings → Connectors → Add custom connector, the MCP URL above,
and under Advanced settings the OAuth Client ID `parcel-claude` (no secret).

**Anything that can only send a header:** a Parcel API token,
`Authorization: Bearer pcl_…`. Such a token pauses after 30 days without a web
sign-in, like on the CLI; on MCP a paused token just reads as invalid, so sign
in on the web and it works again.

You approve `parcel:read` and `parcel:write` once on the sign-in's consent
screen, and the agent acts as you (its comments show "via MCP"); disconnect
from slauth's connected apps. An MCP request is at most 16 MiB; publish bigger
sites with the CLI.

## Run it locally

Requires Docker, [mise](https://mise.jdx.dev), [bun](https://bun.sh),
[uv](https://docs.astral.sh/uv/) and [prox](https://github.com/charliek/prox),
plus a sign-in provider: the author's slauth (not public yet), any OIDC
provider such as the local Keycloak in [dev/keycloak](dev/keycloak/README.md),
or `AUTH_MODE=dev` for a fixed local user.

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

## License

[MIT](LICENSE)

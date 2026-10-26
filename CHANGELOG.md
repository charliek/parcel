# Changelog

## [Unreleased]

### Changed
- **Search falls back to any word.** `q` (`GET /api/v1/parcels`, the web
  app's search, `parcel list -q`, MCP `list_parcels`): every word must
  match; when nothing does, parcels matching any of the words are returned,
  best first, and marked relaxed; operators (quotes, `-word`, `or`) turn that
  off, and so does a query of one word or one token (`plan-002`, `e-mail`).
  Relaxed results rank by how many of the words they have, then relevance,
  and their snippets mark every matched word. `ParcelList` and `TagList`
  gain `relaxed: true` (absent when false); `GET /api/v1/tags` takes
  `mode=auto|exact|relaxed` (default `auto`: as the list decides), and the
  web app's facet counts follow the list's mode. The MCP `list_parcels`
  (every `which`) and `list_tags` pages carry `relaxed: true` too. New
  feature `search-relaxed` in `/api/v1/info`.
- `parcel list -q` prints "No exact matches; showing parcels that match any
  of the words." on stderr above a relaxed list (`--json` output is the
  body, unchanged but for the flag); the dashboard shows the same notice.
  Older CLIs (and their skills) show relaxed results without the notice:
  they ignore the new field.
- After uploading a new version in the web app, the viewer lands on the
  parcel's canonical address (`/@handle/slug` when it has a slug), as a new
  parcel does, also when the upload started from a pinned old version.

## v0.2.0

The CLI and both plugins for the server's tags, slugs, search and pages
(server `v2026.10.25`, which has the full list). Against an older server the
CLI refuses the new flags with "this server doesn't support … yet" instead of
silently ignoring them; everything from v0.1.2 keeps working.

### CLI (`parcel`)
- **Refs everywhere:** any `<ref>` takes an id, a `/p/<id>[/v/<n>]` or
  `/@handle/slug?v=&page=#fragment` URL, `@handle/slug`, or one of your own
  slugs.
- **Organize:** `publish --tag/--kind/--slug/--project` (inside a git repo a
  new parcel gets `project:<origin repo>` automatically; `--no-auto-tags`,
  `--no-source`), `--slug` upserts so republishing from any machine versions
  the same parcel; `update --add-tag/--remove-tag/--set-tags/--slug/
  --archive/--unarchive`; new `tag`, `archive`, `unarchive`, `tags`
  (`rename`, `delete`) and `handle`.
- **Find:** `list --tag/--project/--kind`, `-q` full text with a snippet
  under each match, `--all`, `--archived`/`--archived-only`.
- **Link and pages:** `link <ref> [--page PATH] [--markdown]` and
  `open --page` print the clean (canonical) URL; `info` shows tags, slug,
  archive state, pages with titles and the version's source (owner only).
- Each publish records a credential-free source (repo, branch, commit,
  host, CLI version); the server shows it only to the owner.

### Claude Code plugins
- `parcel`: the skill teaches organizing and finding parcels (tag
  conventions and kinds, slugs for plans, look before publishing, how
  search matches).
- `parcel-mcp`: the skill covers the 25 MCP tools, including `list_tags`,
  `rename_tag`, `delete_tag`, slug upserts and refs.

## v2026.10.25

Tags, slugs, archive, full-text search and pages (PLAN.md, "Tags, slugs,
archive and search (v0.5)"). Everything is additive: an older CLI keeps
working against the new server, and the server deploys before the CLI.

### Added
- **Tags and archive.** Up to 20 free-form tags per parcel (the conventions
  `project:<repo>` and `kind:<plan|discovery|research|report|review|dashboard|
  prototype|notes>`; others are topics), `GET /api/v1/tags` (counts over the
  parcels a filter selects), `POST /api/v1/tags/rename` (merges) and
  `/tags/delete`, and `archived` on `PATCH` (archived parcels stay viewable by
  URL; a new version un-archives). Migration 0006.
- **Unified, searchable listing.** `GET /api/v1/parcels` takes `scope`
  (`mine`, `shared`, `all`), repeated `tag` (all of them), `archived` and `q`:
  full text over title, description, tags and the entry page's text
  (stemmed, every word must appear; quotes, `or`, `-word`), with ids, titles,
  descriptions and slugs also matched by substring. Results are ranked and
  carry a `match` snippet as text segments. Page text is extracted after a
  publish commits and swept up by a backfill loop every minute;
  **`parcel-server reindex`** drains the backlog at once (run it after the
  deploy that adds search). `/api/v1/shared` is unchanged.
- **Handles, slugs and clean URLs.** `/@<handle>/<slug>` next to `/p/<id>`
  (which works forever). Each user has a handle (`PATCH /api/v1/me`; old
  handles stay reserved and keep resolving), a parcel an optional slug
  (`<repo>/<plan-name>`, up to four segments), and a renamed slug redirects.
  `GET /api/v1/parcels/by-slug`; a miss and a private parcel are the same 404.
  `ParcelSummary` gains `tags`, `archived_at`, `slug`, `owner_handle` and
  `canonical_url`; `url` stays `<app>/p/<id>`. Migration 0007.
- **Publish upsert.** `slug` on a publish creates the parcel the first time
  and adds a version to it after that, from any machine, following the
  owner's redirects; two first publishes of one slug make one parcel.
  `PublishResult.created`; `base_version` is honoured.
- **Version source.** Each version records where it came from (repository
  without credentials, branch, commit, machine, client), visible to the owner
  only, never holding a credential.
- **Pages within a parcel.** A version's `.html`/`.md` files are listed with
  their titles (`pages`, `FileEntry.page_title`, migration 0008); the viewer
  has a Pages menu and `?page=` deep links, `view?path=` mints a grant for one
  page, and a non-entry Markdown file renders as a styled page when opened as
  one (`fetch()` still gets the raw text; `Vary: Sec-Fetch-Dest, Accept`). The
  page helper reports the URL fragment.
- **`ServerInfo.features`** (`tags`, `search`, `archive`, `slugs`, `pages`),
  so a newer client refuses a parameter an older server would ignore.
- **CLI.** `parcel publish --tag/--kind/--project/--slug/--no-auto-tags/
  --no-source` (inside a git repository a new parcel gets `project:<repo>`),
  `parcel list --tag/--project/--kind/-q/--shared/--all/--archived/
  --archived-only` (snippets under matching rows), `parcel update
  --add-tag/--remove-tag/--set-tags/--slug/--archive/--unarchive`, and the
  new `tags` (`rename`, `delete`), `tag`, `archive`, `unarchive`, `link`
  (`--page`, `--markdown`), `handle` commands; `open --page`. Every REF takes
  an id, a `/p/<id>` or `/@handle/slug` URL, `@handle/slug` or one of your
  slugs. `PARCEL_PROJECT` names the project.
- **MCP** (25 tools, was 22): `list_tags`, `rename_tag`, `delete_tag`;
  `list_parcels` takes `which`, `query`, `tags` and `archived`;
  `publish_parcel` takes `tags` and `slug` (upsert); `update_parcel` takes the
  organization fields; `get_parcel` returns `pages`; every `parcel` argument
  takes the ref forms. Output stays a superset of what existing clients read.
- **Web app.** Dashboard with a Mine/Shared/All toggle, facet sidebar
  (projects, kinds, tags with filter-scoped counts), token search with
  autocomplete, filter chips, snippets, a show-archived toggle and shareable
  URL state; tags and slug in Edit details and Upload; a Tags page to rename,
  merge and delete; a handle dialog; viewer badges, canonical link, Source
  block for the owner.
- **Skills.** The CLI skill and the `parcel-mcp` skill teach "organize and
  find": look before publishing, tag `project:`/`kind:`, slug plans
  `<repo>/<plan-name>`, `parcel link`, archive, pages and how search matches.

### Changed
- **stridelabs-rust v0.6.0 → v0.10.0** (all crates; adds stridelabs-postgres).
  `/mcp` answers 503 (not 401) for a token whose `kid` the key cache hasn't
  seen yet; JWKS sets are never served past their TTL. Advisory fixes in the
  lock: tokio 1.54.1 (RUSTSEC-2026-0386), rustls-webpki, webpki roots,
  ctutils, and `zip` 9 via utoipa-swagger-ui 10.0.2 (RUSTSEC-2026-0367/8/9).
- **Operator-facing:** `DATABASE_URL` is checked before sqlx parses it, so a
  bad URL's error no longer quotes it; a **`channel_binding` parameter is
  refused** at startup (sqlx can't do channel binding; store the URL without
  it, as deploy/README.md says), and `migrate` and `serve` with
  `RUN_MIGRATIONS` **refuse a Neon pooled URL** (use the direct one for
  migrations) with a message that doesn't contain the URL. Check the
  `DATABASE_URL` secrets before deploying.
- Logging silences sqlx's pool and URL-option targets, which can print raw
  errors and URL parameter values; migration and readiness errors are
  rendered without row or URL text.
- Mixed-revision rollover: a publish by the previous revision still
  invalidates search text and un-archives (database triggers); in the window
  where both revisions run, a transaction can fail with a deadlock (40P01)
  and the client retries.
- Dev builds use line-tables-only debug info (none for dependencies).

### Fixed
- Refs: `?v=` pins only clean (`/@handle/slug`) and slug refs; an id URL is
  pinned by `/v/<n>`, as the web app reads it.
- The web app refreshes cached slug resolutions after a delete, publish, slug
  or handle change, so a reused name never opens the parcel that held it.
- The functional suite rebuilds the released-CLI worktree when CI's cache
  restored its folder without the worktree.

### Security
- `make lint` and CI now run `cargo deny --locked check` (advisories, yanked
  and unsound crates over the whole graph) and `scripts/crypto-guard.sh`
  (neither `ring` nor `rsa` anywhere in the graph).
- The test harness refuses a `DATABASE_URL` that isn't local before it drops
  or creates schemas, and test pools keep their own `search_path` when
  `DATABASE_URL` is set (`make test` runs that check a second time).
- Snippets and page text are rendered as text everywhere (React text nodes,
  the CLI's `printable()`); Biome's `noDangerouslySetInnerHtml` is an error.

## v0.1.2

The first release of the `parcel` CLI and the two Claude Code plugins
(`brew install charliek/tap/parcel`, or `sudo apt install parcel` from
apt-charliek; `/plugin marketplace add charliek/parcel`). Server releases keep
their own date tags; see RELEASING.md, "Two release streams". (v0.1.1 was
tagged but never published: its version bump missed `openapi.json`, so CI
failed and the release workflow refused it; `update-version.sh` bumps it now.)

### CLI (`parcel`)
- Sign in with a browser device flow (`login`, `logout`, `whoami`), then
  `publish` a file or folder as a new parcel or a new version of one (merge or
  replace, `--base-version` conflict checks); `list` (yours or `--shared`),
  `info`, `cat`, `pull`, `versions`, `restore`, `update`, `delete`, `open`.
- Sharing by email (`share`, `unshare`, `shares`), comments left in the viewer
  (`comments list|add|reply|resolve|reopen|delete`) and page data (`data
  list|get|set|update|delete|clear`). `--json` prints the API's response.
- `skill install` writes the bundled agent skill for Claude Code, craze and
  opencode. macOS and Linux, amd64 and arm64; rustls on aws-lc-rs with the
  OS trust store plus Mozilla roots.

### Claude Code plugins
- `parcel-mcp`: Parcel's MCP server (`https://parcel.stridelabs.ai/mcp`,
  OAuth client `parcel-claude-code`, callback port 3118) and a skill for the
  publish → comment → revise loop. No CLI needed.
- `parcel`: the CLI skill; needs the CLI.

## v2026.10.18

### Changed
- Migration 0005 drops `users.sub`, the pre-provider identity column that
  0003 copied into `user_identities` and kept only for the v0.3 rollover. It
  refuses to run if a user with a legacy `sub` has no identity.

## v2026.10.17.3

### Added
- MCP tools for the feedback loop and page data, so MCP is as complete an
  agent interface as the CLI: `list_comments`, `add_comment`,
  `reply_comment`, `resolve_comments` (with a reply), `reopen_comment`,
  `delete_comment`; `list_data`, `get_data`, `set_data`, `update_data`,
  `delete_data`; and `restore_version`. Comments written through MCP are
  recorded and shown "via MCP" (migration 0004); comment bodies are labeled
  as other people's words wherever an agent reads them.
- Claude Code plugins in this repo's marketplace: `parcel-mcp` (the MCP
  server with its OAuth client and callback port, plus a skill for the
  publish → comment → revise loop) and `parcel` (the CLI skill). Both release
  with the CLI (`update-version.sh`, `release-cli.yml`).

## v2026.10.17.2

### Added (v0.4: MCP server)
- An MCP server at `<app>/mcp` (`PARCEL_MCP_ENABLED`, `PARCEL_MCP_PROVIDER`,
  `PARCEL_MCP_JWKS_URL`) on the new shared crate stridelabs-mcp
  (stridelabs-rust v0.6.0, which all stridelabs-* pins move to): stateless
  Streamable HTTP, RFC 9728 metadata, slauth access tokens minted for the
  resource (or `pcl_` tokens). Tools: `whoami`, `list_parcels`, `get_parcel`,
  `read_file`, `publish_parcel`, `update_parcel`, `delete_parcel`,
  `list_shares`, `share_parcel`, `unshare_parcel` — the REST API's operations
  and rules, one audit line per call. Connect claude.ai with client ID
  `parcel-claude`, Claude Code with `parcel-claude-code` (README).

### Changed (maintenance)
stridelabs-rust v0.5.0 and the dependency majors that come with it (their
types are in those crates' signatures); no API change beyond what is listed.
- stridelabs-* v0.4.0 → v0.5.0, reqwest 0.12 → 0.13, sqlx 0.8 → 0.9,
  jsonwebtoken 9 → 11, tower-http 0.6 → 0.7, utoipa 5 → 6 (utoipa-axum 0.3,
  utoipa-swagger-ui 10), rand 0.9 → 0.10.
- One crypto provider, aws-lc-rs, instead of ring: rustls (outbound HTTPS,
  Postgres TLS) and jsonwebtoken (ID tokens, GCS service-account assertions).
  ring is compiled nowhere, and the lock no longer has `rsa`
  (RUSTSEC-2023-0071) or the yanked `zip 3.0.0`.
- The server's outbound HTTPS trusts the OS store (the image's CA bundle)
  through rustls's platform verifier, no longer bundled roots as well.
  Postgres TLS still verifies against bundled roots (`sslmode=verify-full`).
  The CLI trusts the OS store plus bundled Mozilla roots, as before.
- The built SPA answers with strong `ETag`s, so reloading the app shell
  (`Cache-Control: no-cache`) revalidates to a 304. A file path with a
  trailing slash now gets the app shell instead of the file.
- `openapi.json` (utoipa 6): responses without a description now carry
  their status's reason phrase (OpenAPI 3.1 requires one; utoipa 6 drops an
  empty one), and an optional `$ref` lists the `$ref` before `null`. Paths,
  schemas and types are unchanged.
- Logs: the request span's id field is `requestId` (was `request_id`), the
  key Cloud Logging lines correlate on.

### Added
- `LOG_FORMAT=cloud`: Google Cloud Logging lines (`severity`, a top-level
  `message`, `requestId`). Opt-in: production stays `json`, because Cloud
  lines carry no span fields, so the per-request line would lose its method
  and path.

## v2026.10.17

- The sign-in button reads "Stride Labs" in production (`AUTH_SLAUTH_LABEL`).
- Dependency advisories patched (lock only): h2 0.4.20 (RUSTSEC-2026-0258),
  rustls 0.23.46, quinn-proto 0.11.19; chacha20 off a yanked release.

## v2026.10.16

First production release: `parcel.stridelabs.ai` (app) and
`parcelcontent.stridelabs.ai` (content) on Cloud Run + Neon + GCS. The repo
is public on GitHub; CI runs the Rust, functional, frontend and Playwright
suites and builds the production image. Releases go through `/deploy:build`
(date tag → `release.yml` builds the image) and `/deploy_app` (`deploy.yml`:
migrate job, then the service), authenticated by Workload Identity Federation.

### Added (v0.3: sign-in providers and sharing)
- Server-side OIDC sign-in through one or more providers at once (slauth, Okta,
  Keycloak, …): `AUTH_PROVIDERS` + `AUTH_<ID>_*`, per-provider allowed email
  domains, verified emails required (or `TRUST_EMAIL`), optional provider
  logout. The browser holds a signed session cookie; no provider tokens are
  stored. A person signing in through two providers with one email is one user.
- Sharing: `private` parcels can be shared with people by email as viewer or
  commenter (even before they first sign in); new `internal` visibility for
  everyone signed in; `PARCEL_ALLOW_PUBLIC=false` turns public links off.
  Unsharing or narrowing visibility stops open pages at once. "Shared with me"
  (`GET /api/v1/shared`). `parcel share`/`unshare`/`shares`, `parcel list
  --shared`, `--internal`.
- Sign out everywhere; `users.disabled_at` blocks a person at once;
  `PARCEL_TOKEN_LOGIN_MAX_AGE_DAYS` pauses API tokens of people who haven't
  signed in on the web lately (so offboarding at the provider reaches them);
  `PARCEL_SESSION_TTL_SECS`.
- A local Keycloak (`docker compose --profile keycloak`) for trying several
  providers.

### Changed (v0.3)
- `AUTH_MODE=slauth` is now `oidc` (the old value still works);
  `SLAUTH_JWKS_URL` and `SLAUTH_AUDIENCE` are no longer read. The API no
  longer accepts slauth access tokens as bearers: the web app uses its
  session cookie, the CLI and agents `pcl_` tokens.
- `/api/v1/info`'s `auth` is now `{mode, providers}`, plus `allow_public`.

### Added (v0.2: comments, live updates, page data)
- Live updates: the viewer polls `/api/v1/parcels/{id}/activity` and reloads
  the page when a new version is published.
- Comments: threads numbered per parcel, optionally pinned to a spot on the
  page from the viewer's comment mode; replies, resolve/reopen, edit, delete.
  The owner, and signed-in users on public parcels, can comment. `parcel
  comments` (list/add/reply/resolve/reopen/delete) for agents.
- Page data: per-parcel JSON collections (`/api/v1/parcels/{id}/data`, and
  `/d/{token}` on the content origin for the page itself). Viewers read; only
  the owner writes. `parcel data` (list/get/set/update/delete/clear).
- The page helper script, added to every HTML page on the content origin:
  in-memory `localStorage`/`sessionStorage`/`document.cookie` so sandboxed
  pages don't throw, the `window.parcel` API, and the comment-mode bridge.
- `PARCEL_MAX_DATA_BYTES` (10 MiB per parcel) and `PARCEL_MAX_DATA_VALUE_BYTES`
  (256 KiB per value).

### Added
- `parcel-server`: slauth-authenticated API for publishing HTML parcels with
  supporting files, versions (merge/replace, optimistic concurrency, restore),
  visibility, configurable size/count/quota limits, a CSP-sandboxed content
  origin behind HMAC capability URLs, Markdown entry pages, content-addressed
  blob storage (filesystem, GCS) with grace-period GC, `pcl_` API tokens and a
  device-flow CLI login. OpenAPI document committed and pinned; Swagger UI in
  development.
- `parcel` CLI: login/logout/whoami, publish (file or folder), list, info, cat,
  pull, update, versions, restore, delete, open, `skill install`.
- The `parcel` agent skill for Claude Code, craze and opencode.
- React web app: dashboard, sandboxed viewer, uploads, tokens, CLI approval.
- Functional (pytest) and Playwright suites; Docker image; Cloud Run deploy
  templates and workflows (prepared, not yet run).

# parcel CLI reference

`parcel <command> --help` is authoritative; this is the overview an agent needs.
`<ref>` names a parcel: see [Refs](#refs).

## Global options and environment

| Flag | Env | Meaning |
|---|---|---|
| `--server <URL>` | `PARCEL_SERVER` | Server to talk to. Default: the profile's server, else `https://parcel.stridelabs.ai` |
| `--profile <NAME>` | `PARCEL_PROFILE` | Config profile (default `default`) |
| — | `PARCEL_TOKEN` | API token; overrides the saved one |
| — | `PARCEL_CONFIG_DIR` | Directory holding `config.toml` (default `$XDG_CONFIG_HOME/parcel` or the OS config dir) |
| `--project <NAME>` (`publish`) | `PARCEL_PROJECT` | Name for a new parcel's `project:<name>` tag (default: the current directory's git repository) |

The config file is `config.toml`, mode 0600:

```toml
[profiles.default]
server = "https://parcel.stridelabs.ai"
token = "pcl_…"
```

A saved token is only ever sent to the server it was issued by; with
`--server` pointing elsewhere the CLI behaves as signed out. The CLI refuses to
read a token from a config file other users can read (`chmod 600` fixes it).

## Refs

Every `<ref>` argument takes any of:

| Form | Example |
|---|---|
| Parcel id | `abcdefghijkm` |
| Viewer URL | `https://parcel.stridelabs.ai/p/abcdefghijkm`, optionally ending `/v/<n>` to pin a version |
| Clean URL | `https://parcel.stridelabs.ai/@ada/myrepo/latency-plan`, with optional `?v=<n>`, `?page=<path>` and `#fragment` |
| Handle and slug | `@ada/myrepo/latency-plan` |
| One of your own slugs | `myrepo/latency-plan` |

A slug ref costs one lookup (`GET /parcels/by-slug`), after which the command
runs by id; a private parcel you can't see and a missing one fail the same way
(exit 4). A pinned version, page or fragment comes along from the ref, and an
explicit `--version` / `--page` wins over it. A slug of 10–14 letters from the id
alphabet in one segment (`management`) reads as a malformed id when bare: write
`@<handle>/management` or the URL. Slugs never have a segment that looks like an
id, or `p` or `v`.

## Commands

| Command | Does |
|---|---|
| `parcel login [--no-browser]` | Device login: prints a code and URL, opens the browser, waits for approval, saves the token. Interactive — the user runs it |
| `parcel login --with-token` | Read a `pcl_…` token (minted on the web app's Tokens page) from stdin, verify it, save it |
| `parcel logout` | Revoke this profile's token on the server and remove it locally |
| `parcel whoami [--json]` | Signed-in email, storage used and quota |
| `parcel publish <PATH> [flags]` | Create a parcel, or a new version with `--parcel` (below) |
| `parcel list [-q T] [--tag T]… [--project P] [--kind K] [--shared\|--all] [--archived\|--archived-only] [--limit N] [--offset N] [--json]` | Your parcels (or shared with you, or both): id, title, visibility or owner and role, version, size, updated, comments, then TAGS and SLUG columns when any row has them. `-q` is full text and prints the matching text under each row; archived rows say `(archived)` |
| `parcel tags [--prefix P] [--shared\|--all] [--json]` | The tags on your parcels with counts (counts over the parcels in scope), most used first |
| `parcel tags rename FROM TO [--json]` | Rename a tag on every parcel you own; into an existing tag the two merge |
| `parcel tags delete TAG [--json]` | Remove a tag from every parcel you own |
| `parcel tag <ref> [--add T…] [--remove T…] [--json]` | Add or remove tags on one parcel; then prints its tags |
| `parcel archive <ref>` / `parcel unarchive <ref>` `[--json]` | Hide a parcel from lists, tag counts and search (it stays openable by URL), or bring it back |
| `parcel link <ref> [--page PATH] [--markdown] [--json]` | Print the link to hand over: the clean URL when the parcel has a slug, else the `/p/<id>` URL (with the ref's `?v=`, and `?page=` from `--page`). `--markdown` prints `[title](url)` |
| `parcel handle [NEW] [--json]` | Show your handle (the `@handle` in clean URLs), or change it; the old one keeps working in links and nobody else can take it |
| `parcel info <ref> [--version N] [--json]` | Metadata, versions (newest first) and files |
| `parcel cat <ref> [PATH] [--version N]` | Raw bytes of one file (default: the entry page) to stdout |
| `parcel pull <ref> [DIR] [--version N] [--force]` | Download a version's files into DIR (default `./<id>`); refuses to overwrite without `--force` |
| `parcel update <ref> [--title T] [--description D] [--private\|--internal\|--public] [--add-tag T]… [--remove-tag T]… [--set-tags a,b] [--slug S] [--archive\|--unarchive] [--json]` | Change metadata (files change only through publish). `--set-tags ""` removes every tag; `--slug ""` removes the slug (the old one keeps redirecting). There is no bare `--tag` here, so adding and replacing can't be confused |
| `parcel share <ref> <EMAIL>... [--role viewer\|commenter] [--json]` | Share with people by email (owner only); then prints who has access |
| `parcel unshare <ref> <EMAIL>... [--json]` | Stop sharing with them; then prints who still has access |
| `parcel shares <ref> [--json]` | Who the parcel is shared with: email, role, name (or `not signed in yet`) |
| `parcel versions <ref> [--json]` | Version history |
| `parcel restore <ref> --version N [--json]` | Publish a copy of version N as the newest version |
| `parcel delete <ref> [--yes]` | Delete the parcel and every version. Prompts on a terminal; needs `--yes` otherwise |
| `parcel open <ref> [--page PATH] [--json]` | Print the viewer URL and open it in a browser (`--json` prints `{url}`) |
| `parcel comments list <ref> [--status open\|resolved\|all] [--json]` | Comment threads (default: open ones), with page, element, author and replies |
| `parcel comments add <ref> <TEXT> [--version N] [--json]` | A general comment about the whole parcel |
| `parcel comments reply <ref> <N> <TEXT> [--json]` | Reply to thread `#N` |
| `parcel comments resolve <ref> <N>... [--reply TEXT]` | Resolve threads; with `--reply`, reply to each one first |
| `parcel comments reopen <ref> <N>...` | Reopen resolved threads |
| `parcel comments delete <ref> <N>` | Delete a comment (a thread takes its replies with it) |
| `parcel data list <ref> [COLLECTION] [--json]` | Collections with item counts and usage; or every item of one collection |
| `parcel data get <ref> <COLLECTION> <KEY> [--json]` | One item's value, pretty-printed JSON (`--json`: the whole item) |
| `parcel data set <ref> <COLLECTION> <KEY> <VALUE> [--string] [--json]` | Create or replace an item (owner only) |
| `parcel data update <ref> <COLLECTION> <KEY> <PATCH> [--json]` | Merge a JSON object into an item, creating it if needed (owner only) |
| `parcel data delete <ref> <COLLECTION> <KEY>` | Delete one item (owner only) |
| `parcel data clear <ref> <COLLECTION>` | Delete every item in a collection (owner only) |
| `parcel skill install [--agents] [--project] [--dir DIR] [--force]` | Write this skill to `~/.claude/skills/parcel/` (`--agents`: also `~/.agents/skills/parcel/`; `--project`: under `./` instead; `--dir`: into DIR) |
| `parcel skill show` / `parcel skill path` | Print SKILL.md / where it installs |
| `parcel config show` / `parcel config set-server <URL>` | Show settings (never the token) / change the profile's server |

### publish flags

| Flag | Meaning |
|---|---|
| `<PATH>` | A file (published under its file name; the entry page for a new parcel) or a directory (walked recursively) |
| `--title`, `--description` | Metadata. Title defaults to the entry's `<title>` / first heading, else its file name |
| `--private` / `--internal` / `--public` | Visibility (new parcels are private); see below |
| `--entry <path>` | Entry page path inside the parcel (`.html`, `.htm`, `.md`, `.markdown`) |
| `--label <text>` | Note stored with this version |
| `--tag T` | Add a tag (repeatable). On a new version of an existing parcel the tags are added to its set |
| `--kind K` | `--tag kind:K`: `plan`, `discovery`, `research`, `report`, `review`, `dashboard`, `prototype`, `notes` |
| `--project NAME` | The new parcel's `project:NAME` tag instead of the git repository's (env `PARCEL_PROJECT`; the flag wins) |
| `--no-auto-tags` | No automatic `project:` tag, whatever its origin |
| `--no-source` | Don't record where this was published from (repository, branch, commit, machine) |
| `--slug S` | Upsert: create the parcel with this slug the first time, add a new version of it (every file replaced; only the fields you pass change) after that, from any machine. Follows your redirects from renamed slugs. Conflicts with `--parcel`, `--mode` and `--remove`; `--base-version N` is allowed and also fails when nothing has the slug yet. Prints `created` in `--json` |
| `--file PUBLISHED=LOCAL` | Add a file at an explicit published path (repeatable; `--file LOCAL` keeps the file name). Overrides a walked file with the same path |
| `--parcel <ref>` (alias `--update`) | Publish a new version of this parcel instead of creating one |
| `--mode merge\|replace` | With `--parcel`. Default: `replace` for a directory, `merge` for a single file or bare `--file`/`--remove` |
| `--remove <path>` | With `--parcel`, merge mode: drop a path from the previous version (repeatable) |
| `--base-version N` | With `--parcel`: fail (HTTP 409) unless the current version is still N |
| `--open` | Open the result in a browser |
| `--json` | Print the `PublishResult` JSON |

Inside a git repository, a **new** parcel gets the tag `project:<repo>`
(the origin remote's name, else the main checkout's folder; the current
directory's repository, so publish from where you work, not from a scratch
directory). It is applied only when the call creates the parcel, so a project
tag you remove later stays removed, and it is dropped first if tags would exceed
20. Every publish also records a **source** for the version (repository
without credentials, branch, commit, machine, `parcel-cli/<version>`), which only
the owner sees. Without `git`, outside a repository or after a 2 s timeout, no
project tag and no repository fields are sent.

### visibility and sharing

| Visibility | Who can open it |
|---|---|
| `private` (default) | The owner and the people it is shared with |
| `internal` | Anyone signed in to this Parcel server (every one of them can comment) |
| `public` | Anyone with the link, signed in or not. Signed-out viewers see no comments |

Servers can turn public links off: `--public` is then refused (`public links
are turned off on this server…`; `publish` refuses before uploading), and
parcels made public earlier act and report as `internal`. Use `--internal`,
or keep it private and share it.

Sharing is by email and only the owner does it. Emails are matched
case-insensitively against the address the person signs in with, so you can
share before they have an account: access starts at their first sign-in.
Roles: `viewer` (the default) sees the parcel, its comments and its data;
`commenter` also comments. Neither can publish, change, share or write data.
On an internal or public parcel everyone signed in is already a commenter.

- `share` with an existing email changes that person's role. It refuses
  malformed addresses (exit 2) before sending, your own address, and a
  domain no sign-in provider of this server accepts.
- `unshare` takes effect at once, including pages the person has open.
  An email the parcel wasn't shared with gets a `note:` on stderr, not an
  error.
- Output after `share`/`unshare`, and from `shares`:

```
Shared with 2 people:
EMAIL            ROLE       NAME
ada@example.com  commenter  Ada Lovelace
bob@example.com  viewer     not signed in yet
```

### comments

Comments come from the web viewer's comment mode (or from `comments add`). Threads are numbered per parcel, `#1`, `#2`, …; replies take numbers too, and numbers are never reused. Use a **thread's** number for `reply`, `resolve` and `reopen`.

- Who can: see comments — the owner, the people the parcel is shared with, and anyone signed in when it is internal or public (signed-out viewers see none; the CLI needs a login for every `comments` command). Add them — all of those except people shared as `viewer`. Edit — the author. Resolve or reopen — the owner or the thread's author. Delete — the author or the owner.
- `<TEXT>` and `--reply` accept `-` to read the text from stdin (handy for multi-line replies). Text is trimmed; 1–4000 characters.
- `comments add` comments on the version in a `/v/<n>` URL, else `--version`, else the current one.
- `comments resolve 1 3 --reply "Fixed in v4: …"` posts the reply to #1, resolves #1, then does #3. It stops at the first failure; threads already handled stay resolved and are listed on stdout.
- Comments made through the CLI (a `pcl_` token) show as `via cli`.

Human output of `comments list`:

```
2 open threads on abcdefghijkm

#1 · v2 · index.html › "Revenue by region" · Sam · 3h ago
    Make the bars blue and label the axis.
    #4 · you (owner) via cli · 5m ago
        Is navy OK?

#3 · v2 · about.html › main > p:nth-of-type(2) · Sam · 2h ago
    Typo: "recieve".
```

The location is the page path, then the clicked element's text (or its CSS selector when it had none); `general` marks a comment on the whole parcel. Resolved threads end with `· resolved <when>`.

### data

A parcel's data is JSON its pages keep between visits (`window.parcel.data`), in named collections of keyed items, shared by every version. Anyone who can view the parcel reads it (public parcels need no login); only the owner changes it.

- `<VALUE>` / `<PATCH>`: inline JSON (`'{"done": true}'`, `42`, `'"text"'`), `@path/to/file.json`, or `-` for stdin. Sent minified, keys in the order written.
- `set --string` stores the text as a JSON string instead of parsing it (a single trailing newline from a file or stdin is dropped): `parcel data set <ref> notes intro @intro.md --string`.
- `update` takes a JSON object and merges it (RFC 7386): fields replace, nested objects merge, `null` deletes a field.
- Names (collections and keys) are 1–128 of `A–Z a–z 0–9 _ . : -`, starting with a letter or digit; the CLI rejects others (exit 2) before sending.
- Limits: one value 256 KiB, a parcel 10 MiB (each item counts its JSON plus 128 bytes); see `limits` in `parcel whoami --json`. `parcel data list <ref>` shows usage against the limit.
- `data list <ref> <collection>` fetches every page; with `--json` it prints them merged as one `{"items": [...]}`.
- `data get` of a missing item exits 4.

## JSON shapes

`--json` prints the server's response unchanged. Timestamps are RFC 3339 UTC
strings, sizes are bytes.

**ParcelSummary** — one parcel (in lists, `publish`, `update`); `visibility` is `private`, `internal` or `public`:

```json
{
  "id": "abcdefghijkm",
  "title": "Q3 latency report",
  "description": "",
  "visibility": "private",
  "current_version": 3,
  "entry": "index.html",
  "file_count": 4,
  "total_bytes": 182340,
  "created_at": "2026-10-01T12:00:00Z",
  "updated_at": "2026-10-15T09:30:00Z",
  "url": "https://parcel.stridelabs.ai/p/abcdefghijkm",
  "open_comments": 2,
  "tags": ["kind:report", "project:myrepo"],
  "archived_at": null,
  "slug": "myrepo/latency-report",
  "owner_handle": "ada",
  "canonical_url": "https://parcel.stridelabs.ai/@ada/myrepo/latency-report"
}
```

`url` is always `<server>/p/<id>`; **`canonical_url` is the link to show and share**
(the clean URL when there is a slug, else equal to `url`). `tags` are sorted;
`slug` and `archived_at` are `null` when unset; `owner_handle` is left out for
signed-out callers of a parcel without a slug.

**VersionSummary**: `{"version", "entry", "label" (string or null), "file_count", "total_bytes", "created_at"}`, plus `"source"` for the owner only: `{"repo", "branch", "commit", "host", "client"}`, each optional.
**VersionDetail**: a VersionSummary's fields plus `"files": [FileEntry]` and `"pages": [PageEntry]`.
**FileEntry**: `{"path", "size_bytes", "content_type", "sha256"}`, plus `"page_title"` on a page that has one.
**PageEntry**: `{"path", "title" (string or null)}`: the version's `.html`, `.htm`, `.md` and `.markdown` files, the entry first, then by path.
**ListMatch** (`match` on a list item, when the query matched page text): `{"segments": [{"text", "hit": bool}]}`; concatenated, the segments are a passage of the page and `hit` marks the matched words. Plain text, never HTML.

| Command | Prints |
|---|---|
| `publish`, `restore` | `PublishResult`: `{"parcel": ParcelSummary, "version": VersionDetail, "created": bool}` — the link is `.parcel.canonical_url`, the new version `.version.version`; `created` is `false` when a `--slug` publish added a version to an existing parcel |
| `list` (yours, or `--all`, or filtered `--shared`) | `ParcelList`: `{"parcels": [ParcelListItem], "total": N}`; a ParcelListItem is a ParcelSummary's fields plus `"match"` (a ListMatch, only when `-q` matched page text) and, on someone else's parcel, `"shared": {"owner_name", "role", "shared_at"}` |
| `list --shared` (plain: no `-q`, `--tag`, `--project`, `--kind` or `--archived*`) | `SharedParcelList` from the legacy `/shared` endpoint: `{"parcels": [{"parcel": ParcelSummary, "owner_name", "role": "viewer"\|"commenter", "shared_at"}], "total": N}` (archived parcels included) |
| `list --shared` with any of those filters | the `/parcels?scope=shared` body: a `ParcelList` as in the `list` row above, **not** the SharedParcelList shape. Scripts that parse `--shared` output must expect both |
| `info` | `ParcelDetail`: `{"parcel": ParcelSummary, "is_owner": bool, "access": "owner"\|"commenter"\|"viewer"\|"public", "owner_name" (signed in only), "versions": [VersionSummary], "files": [FileEntry], "pages": [PageEntry]}` (files and pages of the current version). With `--version N` or a `/v/<n>` URL: that version's `VersionDetail` |
| `tag`, `archive`, `unarchive`, `update` | `ParcelSummary` |
| `tags` | `{"tags": [{"tag", "count"}], "total": N}` (every page is fetched and merged) |
| `tags rename`, `tags delete` | `{"updated": N}`: how many of your parcels changed |
| `link` | `{"url", "canonical_url", "markdown"}` (the `--page` and `?v=` of the ref included) |
| `open` | `{"url"}` (the browser still opens) |
| `handle` | `{"handle", "previous"}` (`previous` only after a change) |
| `shares`, `share`, `unshare` | `ShareList`: `{"shares": [{"email", "role", "name" (once they've signed in), "created_at"}]}` |
| `versions` | `VersionList`: `{"versions": [VersionSummary]}`, newest first |
| `whoami` | `Me`: `{"id", "email", "name", "handle", "usage": {"parcel_count", "version_count", "stored_bytes"}, "limits": {"max_entry_bytes", "max_file_bytes", "max_upload_bytes", "max_files_per_upload", "max_files_per_version", "max_version_bytes", "user_quota_bytes", "max_data_bytes", "max_data_value_bytes"}}` (`user_quota_bytes` 0 = unlimited) |
| `comments list` | `CommentList`: `{"comments": [Comment], "open": N}` — oldest first; `open` counts unresolved threads whatever `--status` is |
| `comments add`, `comments reply` | `Comment`: the whole thread |
| `data list` | `DataSummary`: `{"revision", "usage_bytes", "limit_bytes", "writable", "collections": [{"name", "count", "usage_bytes", "updated_at"}]}`. With a collection: `{"items": [DataItem]}` |
| `data get`, `data set`, `data update` | `DataItem`: `{"collection", "key", "value", "size_bytes", "updated_at"}` |

**Comment** (a thread):

```json
{
  "number": 1,
  "version": 2,
  "author": {"name": "Sam Lee", "is_owner": false, "is_you": false},
  "body": "Make the bars blue and label the axis.",
  "anchor": {"path": "index.html", "selector": "html > body > main > h2:nth-of-type(2)",
             "text": "Revenue by region", "x": 0.42, "y": 0.5},
  "via": "web",
  "resolved": false,
  "resolved_at": null,
  "created_at": "2026-10-15T09:30:00Z",
  "updated_at": "2026-10-15T09:30:00Z",
  "replies": [
    {"number": 4, "author": {"name": "Ada", "is_owner": true, "is_you": true},
     "body": "Is navy OK?", "via": "cli",
     "created_at": "2026-10-15T10:00:00Z", "updated_at": "2026-10-15T10:00:00Z"}
  ]
}
```

`anchor` is absent on a general comment; inside it only `path` is always present. `x`/`y` are the click position within the element's box (0–1). `via` is `web` or `cli`.

### Older servers

The CLI reads the server's `features` (`tags`, `search`, `archive`, `slugs`,
`pages`; from `GET /info`, once per run) before it sends anything an older server
would ignore or refuse. Asking for something the server lacks fails with
`this server doesn't support <tags|slugs|searching shared parcels|archiving|pages> yet
(it is older than this CLI)` rather than returning an unfiltered list. What the CLI
adds on its own (the automatic `project:` tag and the version source) is silently
left out there. A plain `list`, `list -q` on your own parcels, `info`, `cat`,
`pull` and `publish` work against any server.

Errors are not JSON on stdout: they go to stderr as `error: <message>` with a
non-zero exit code (see [troubleshooting.md](troubleshooting.md)).

## Examples

```bash
# Publish, capture the link
url=$(parcel publish report.html --title "Q3 latency" --json | jq -r .parcel.url)

# Revise in place, guarding against concurrent edits
v=$(parcel info "$url" --json | jq .parcel.current_version)
parcel publish report.html --parcel "$url" --base-version "$v" --label "Add p99" --json

# Edit an existing multi-file parcel
parcel pull "$url" ./work && $EDITOR ./work/index.html
parcel publish ./work --parcel "$url" --json

# Pin a version for review
parcel cat "$url/v/2" > v2.html

# Look before publishing, then publish under a slug so republishing updates it
parcel tags --prefix project:
parcel list -q "latency" --tag project:myrepo --tag kind:plan
parcel publish ./plan --slug myrepo/latency-plan --kind plan --tag performance --json | jq -r .parcel.canonical_url
parcel link myrepo/latency-plan --markdown          # [Latency plan](https://…/@ada/myrepo/latency-plan)
parcel link myrepo/latency-plan --page appendix.html
parcel archive myrepo/latency-plan                  # finished: out of lists and search

# Show it to two colleagues, one of whom may comment
parcel share "$url" ada@example.com
parcel share "$url" bob@example.com --role commenter

# Work through feedback: read, fix, republish to the same link, resolve
parcel comments list "$url"
parcel publish ./work --parcel "$url" --label "Address comments" --json
parcel comments resolve "$url" 1 3 --reply "Fixed in v5: legend added, axis labelled"

# Which threads are still open, as data
parcel comments list "$url" --json | jq '.comments[] | {number, page: .anchor.path, element: .anchor.text, body}'

# Feed a dashboard page without republishing
parcel data set "$url" dashboard metrics @metrics.json
parcel data update "$url" dashboard metrics '{"p99_ms": 412, "stale": null}'
parcel data get "$url" dashboard metrics | jq .p99_ms
```

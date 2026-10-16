# parcel CLI reference

`parcel <command> --help` is authoritative; this is the overview an agent needs.
`<ref>` is a 12-character parcel id (`abcdefghijkm`) or a viewer URL
(`https://parcel.stridelabs.ai/p/abcdefghijkm`, optionally ending `/v/<n>` to
pin a version).

## Global options and environment

| Flag | Env | Meaning |
|---|---|---|
| `--server <URL>` | `PARCEL_SERVER` | Server to talk to. Default: the profile's server, else `https://parcel.stridelabs.ai` |
| `--profile <NAME>` | `PARCEL_PROFILE` | Config profile (default `default`) |
| — | `PARCEL_TOKEN` | API token; overrides the saved one |
| — | `PARCEL_CONFIG_DIR` | Directory holding `config.toml` (default `$XDG_CONFIG_HOME/parcel` or the OS config dir) |

The config file is `config.toml`, mode 0600:

```toml
[profiles.default]
server = "https://parcel.stridelabs.ai"
token = "pcl_…"
```

A saved token is only ever sent to the server it was issued by; with
`--server` pointing elsewhere the CLI behaves as signed out. The CLI refuses to
read a token from a config file other users can read (`chmod 600` fixes it).

## Commands

| Command | Does |
|---|---|
| `parcel login [--no-browser]` | Device login: prints a code and URL, opens the browser, waits for approval, saves the token. Interactive — the user runs it |
| `parcel login --with-token` | Read a `pcl_…` token (minted on the web app's Tokens page) from stdin, verify it, save it |
| `parcel logout` | Revoke this profile's token on the server and remove it locally |
| `parcel whoami [--json]` | Signed-in email, storage used and quota |
| `parcel publish <PATH> [flags]` | Create a parcel, or a new version with `--parcel` (below) |
| `parcel list [--query T] [--limit N] [--offset N] [--json]` | Your parcels: id, title, visibility, version, size, updated |
| `parcel info <ref> [--version N] [--json]` | Metadata, versions (newest first) and files |
| `parcel cat <ref> [PATH] [--version N]` | Raw bytes of one file (default: the entry page) to stdout |
| `parcel pull <ref> [DIR] [--version N] [--force]` | Download a version's files into DIR (default `./<id>`); refuses to overwrite without `--force` |
| `parcel update <ref> [--title T] [--description D] [--public\|--private] [--json]` | Change metadata (files change only through publish) |
| `parcel versions <ref> [--json]` | Version history |
| `parcel restore <ref> --version N [--json]` | Publish a copy of version N as the newest version |
| `parcel delete <ref> [--yes]` | Delete the parcel and every version. Prompts on a terminal; needs `--yes` otherwise |
| `parcel open <ref>` | Print the viewer URL and open it in a browser |
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
| `--public` / `--private` | Visibility (new parcels are private) |
| `--entry <path>` | Entry page path inside the parcel (`.html`, `.htm`, `.md`, `.markdown`) |
| `--label <text>` | Note stored with this version |
| `--file PUBLISHED=LOCAL` | Add a file at an explicit published path (repeatable; `--file LOCAL` keeps the file name). Overrides a walked file with the same path |
| `--parcel <ref>` (alias `--update`) | Publish a new version of this parcel instead of creating one |
| `--mode merge\|replace` | With `--parcel`. Default: `replace` for a directory, `merge` for a single file or bare `--file`/`--remove` |
| `--remove <path>` | With `--parcel`, merge mode: drop a path from the previous version (repeatable) |
| `--base-version N` | With `--parcel`: fail (HTTP 409) unless the current version is still N |
| `--open` | Open the result in a browser |
| `--json` | Print the `PublishResult` JSON |

### comments

Comments come from the web viewer's comment mode (or from `comments add`). Threads are numbered per parcel, `#1`, `#2`, …; replies take numbers too, and numbers are never reused. Use a **thread's** number for `reply`, `resolve` and `reopen`.

- Who can: see and add comments — the owner, plus any signed-in user when the parcel is public (signed-out viewers see none; the CLI needs a login for every `comments` command). Edit — the author. Resolve or reopen — the owner or the thread's author. Delete — the author or the owner.
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

**ParcelSummary** — one parcel (in lists, `publish`, `update`):

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
  "open_comments": 2
}
```

**VersionSummary**: `{"version", "entry", "label" (string or null), "file_count", "total_bytes", "created_at"}`.
**VersionDetail**: a VersionSummary's fields plus `"files": [FileEntry]`.
**FileEntry**: `{"path", "size_bytes", "content_type", "sha256"}`.

| Command | Prints |
|---|---|
| `publish`, `restore` | `PublishResult`: `{"parcel": ParcelSummary, "version": VersionDetail}` — the link is `.parcel.url`, the new version `.version.version` |
| `list` | `ParcelList`: `{"parcels": [ParcelSummary], "total": N}` |
| `info` | `ParcelDetail`: `{"parcel": ParcelSummary, "is_owner": bool, "versions": [VersionSummary], "files": [FileEntry]}` (files of the current version). With `--version N` or a `/v/<n>` URL: that version's `VersionDetail` |
| `versions` | `VersionList`: `{"versions": [VersionSummary]}`, newest first |
| `update` | `ParcelSummary` |
| `whoami` | `Me`: `{"id", "email", "name", "usage": {"parcel_count", "version_count", "stored_bytes"}, "limits": {"max_entry_bytes", "max_file_bytes", "max_upload_bytes", "max_files_per_upload", "max_files_per_version", "max_version_bytes", "user_quota_bytes", "max_data_bytes", "max_data_value_bytes"}}` (`user_quota_bytes` 0 = unlimited) |
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

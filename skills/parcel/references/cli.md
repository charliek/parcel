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
  "url": "https://parcel.stridelabs.ai/p/abcdefghijkm"
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
| `whoami` | `Me`: `{"id", "email", "name", "usage": {"parcel_count", "version_count", "stored_bytes"}, "limits": {"max_entry_bytes", "max_file_bytes", "max_upload_bytes", "max_files_per_upload", "max_files_per_version", "max_version_bytes", "user_quota_bytes"}}` (`user_quota_bytes` 0 = unlimited) |

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
```

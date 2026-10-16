# Troubleshooting parcel

## Exit codes

| Code | Meaning | What to do |
|---|---|---|
| 0 | Success | — |
| 1 | Error (validation, limits, conflict, server or network failure) | Read the `error:` line; fix and retry only if the fix is clear |
| 2 | Bad arguments (unknown flag, malformed ref, missing `--yes`) | Fix the command; `parcel <command> --help` |
| 3 | Not logged in, or the server rejected the token | Ask the user to run `parcel login` themselves |
| 4 | Not found (parcel, version or file) | Check the id/URL; a private parcel also returns 4 when signed out or signed in as someone else |

Errors print one line (sometimes followed by an indented list) on stderr as
`error: <message>`. With `--json`, stdout carries only successful responses.

## Common errors

| Message (abridged) | Cause | Fix |
|---|---|---|
| `not logged in to <server>; run parcel login` | No saved token for this server/profile | User runs `parcel login` (add `--profile`/`--server` if they use one) |
| `… (profile "default" is logged in to <other>)` | `--server`/`PARCEL_SERVER` points somewhere else than the login | Drop the override, or log in to that server |
| `invalid token (run parcel login to sign in)` | Token revoked or expired | User runs `parcel login` again |
| `<config> holds an API token but is accessible by other users` | `config.toml` mode is wider than 0600 | `chmod 600 <path>` |
| `cannot publish N files: … file type not allowed` | Extension not in the allow-list | Convert (e.g. `.jsx` → bundled `.js`), rename, or leave the file out |
| `… path contains a '.' or '..' segment` / `must be relative` | Bad `--file` published path | Use a relative path like `assets/x.js` |
| `no entry page among the files` | No `.html`/`.md` in a new parcel or a replace | Add `index.html`, or pass `--entry` |
| `--entry "x" is not one of the files being published` | Entry missing from a create/replace upload | Include it, or use `--mode merge` if it already exists in the parcel |
| `over the server's limits` | A file or the total is too big, or too many files | Shrink/compress assets, split data, or publish fewer files |
| `--remove only applies in merge mode` | `--remove` with a folder (replace by default) | Add `--mode merge`, or just leave the file out of the folder |
| conflict / HTTP 409 with `--base-version` | Someone published a newer version | `parcel info <ref>` to see it, merge the changes, publish with the new base |
| `refusing to overwrite existing files (pass --force)` | `pull` target already has those files | Pull into an empty directory, or `--force` if overwriting is intended |
| `refusing to delete <id> without confirmation; pass --yes` | `delete` without a terminal | Confirm with the user, then add `--yes` |
| `could not connect to <server>` | Server down, wrong URL, or offline | `parcel config show` to check the server; retry later |
| `warning: skipping symlink …` | Folder publishes don't follow symlinks | `--file <published>=<target>` to include it explicitly |

## The page is blank or broken

1. **Asset 404s:** paths starting with `/` resolve outside the parcel. Make every `src`, `href` and `fetch` URL relative (`assets/app.js`).
2. **Script stops early:** `localStorage`, `sessionStorage`, `indexedDB` or `document.cookie` threw a `SecurityError` (opaque origin). Wrap them in `try`/`catch` (see authoring.md).
3. **Wrong file got published:** `parcel info <ref>` lists the files and the entry; `parcel cat <ref> <path>` shows exactly what the server has.
4. **External library didn't load:** the CDN was blocked or changed. Inline the library, or vendor it as a supporting `.js` file.
5. **Link opens inside the frame and fails:** add `target="_blank" rel="noopener"` to external links.
6. **Only the old version shows:** the viewer URL tracks the newest version; a URL ending in `/v/<n>` stays on that version. `parcel versions <ref>` confirms what was published.
7. **Markdown looks plain:** a `.md` entry is rendered with a default style; switch to an HTML entry for custom layout.

## Checking state

```bash
parcel config show            # profile, server, whether a token is set (never the token)
parcel whoami                 # account and storage; exit 3 if not logged in
parcel info <ref> --json      # what the server has for a parcel
parcel versions <ref>         # history; restore with: parcel restore <ref> --version N
```

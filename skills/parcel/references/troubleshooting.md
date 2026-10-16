# Troubleshooting parcel

## Exit codes

| Code | Meaning | What to do |
|---|---|---|
| 0 | Success | — |
| 1 | Error (validation, limits, conflict, server or network failure) | Read the `error:` line; fix and retry only if the fix is clear |
| 2 | Bad arguments (unknown flag, malformed ref, missing `--yes`) | Fix the command; `parcel <command> --help` |
| 3 | Not logged in, or the server rejected the token (every `comments` command needs a login) | Ask the user to run `parcel login` themselves |
| 4 | Not found (parcel, version, file, comment or data item) | Check the id/URL; a private parcel also returns 4 when signed out or signed in as someone else |

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

## Comments

| Message (abridged) | Exit | Cause | Fix |
|---|---|---|---|
| `not logged in to <server>` | 3 | Comments need a login, even on public parcels | User runs `parcel login` |
| `parcel not found` | 4 | The parcel is private and you aren't its owner (private parcels are 404 to everyone else, never 403), or the id is wrong | Check the ref and which account `parcel whoami` shows |
| `comment #N not found` | 4 | No such comment (deleted, or a typo) | `parcel comments list <ref> --status all` |
| `reply to the thread's first comment, not to a reply` / `resolve the thread, not a reply` | 1 | `N` is a reply's number | Use the thread's number (the unindented `#N` in `comments list`) |
| `only its author can edit a comment` | 1 | Editing someone else's comment | Reply instead |
| `only the parcel's owner or the thread's author can resolve it` | 1 | Resolving someone else's thread on another person's parcel | Reply instead; the owner resolves |
| `only its author or the parcel's owner can delete a comment` | 1 | Deleting someone else's comment on another person's parcel | — |
| `version N does not exist` | 1 | `comments add --version` (or a `/v/<n>` URL) names a missing version | `parcel versions <ref>` |
| `comment must be 1–4000 characters` | 2 | Empty or too-long text | Shorten it, or split it into replies |
| `a parcel can hold at most 2000 comments` | 1 | The parcel is full | Delete resolved threads |

## Data

| Message (abridged) | Exit | Cause | Fix |
|---|---|---|---|
| `only the owner can change this parcel` | 1 | A data write (`set`, `update`, `delete`, `clear`) on someone else's public parcel. Data is readable by every viewer, writable only by the owner | Ask the owner, or publish your own parcel |
| `parcel not found` on a write | 4 | Someone else's private parcel | — |
| `no item <collection>/<key>` | 4 | `data get` of a missing item | `parcel data list <ref> <collection>` |
| `"x y": key must be 1–128 letters, digits, …` | 2 | Bad collection or key name | Use `A–Z a–z 0–9 _ . : -`, starting with a letter or digit |
| `the value is not valid JSON (…); quote strings … or pass --string` | 2 | `data set` got bare text such as `hello` | Quote it as JSON (`'"hello"'`) or add `--string` |
| `the patch must be a JSON object` | 2 | `data update` got an array, number or string | Send an object, or use `data set` to replace the value |
| `the value is N bytes; a data value can be at most M bytes` | 1 | One value is over the limit (256 KiB by default) | Split it across several keys |
| `this parcel's data would exceed its N byte limit` | 1 | The parcel's data is full (10 MiB by default; each item counts its JSON plus 128 bytes) | `parcel data list <ref>` to see usage; `data delete` / `data clear` what's stale |

From a page, `window.parcel.data` calls reject with an `Error` carrying `.status` and the server's `.message`: **403** usually means this viewer can't write (`parcel.canWrite` is `false`: a visitor, or the owner on a signed-out public link) — show a read-only view instead of retrying; it is also what an expired page link or a parcel made private returns (the message says so; reopening the parcel from Parcel fixes it). **413** is one of the limits above, **400** a bad name, **404** a deleted parcel or an invalid link. (`get` of a missing item resolves to `undefined`, not an error; the browser still logs that request's 404 in the console, which is expected.)

## The page is blank or broken

1. **Asset 404s:** paths starting with `/` resolve outside the parcel. Make every `src`, `href` and `fetch` URL relative (`assets/app.js`).
2. **Script stops early:** `indexedDB` threw a `SecurityError` (opaque origin). Wrap it in `try`/`catch`. (`localStorage`, `sessionStorage` and `document.cookie` don't throw: Parcel's page helper replaces them with in-memory versions.)
3. **Wrong file got published:** `parcel info <ref>` lists the files and the entry; `parcel cat <ref> <path>` shows exactly what the server has.
4. **External library didn't load:** the CDN was blocked or changed. Inline the library, or vendor it as a supporting `.js` file.
5. **Link opens inside the frame and fails:** add `target="_blank" rel="noopener"` to external links.
6. **Only the old version shows:** the viewer URL tracks the newest version; a URL ending in `/v/<n>` stays on that version. `parcel versions <ref>` confirms what was published.
7. **Markdown looks plain:** a `.md` entry is rendered with a default style; switch to an HTML entry for custom layout.
8. **Saved state is gone after a reload:** `localStorage`/`sessionStorage`/cookies are in memory only. Store it with `window.parcel.data` (see authoring.md).
9. **`window.parcel` is undefined:** the page defines its own global `parcel` (rename it), or it's being opened as a local file rather than through Parcel (guard with `window.parcel?.`).
10. **Data writes from the page fail with 403:** only the owner's own, signed-in viewing can write. Check `parcel.canWrite`; an agent can write with `parcel data set`.

## Checking state

```bash
parcel config show            # profile, server, whether a token is set (never the token)
parcel whoami                 # account and storage; exit 3 if not logged in
parcel info <ref> --json      # what the server has for a parcel
parcel versions <ref>         # history; restore with: parcel restore <ref> --version N
parcel comments list <ref> --status all   # every thread, resolved ones too
parcel data list <ref>        # collections, item counts, usage against the limit
```

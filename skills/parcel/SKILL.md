---
name: parcel
description: "Use when the user wants to see, share or hand off something as a web page rendered in a browser at a private link: an HTML page, report, dashboard, chart or data visualization, interactive prototype or mockup, diagram, slide-style summary, or a long document that reads better as a page than as chat. Triggers include 'publish this', 'share a link', 'make it a page', 'show it to me in the browser', 'put it on parcel', 'there's feedback / there are comments on the parcel', 'update the dashboard's data', and any parcel URL (…/p/<id>) or 12-character parcel id the user gives you to read, review, revise, act on comments for, restore or delete. Works through the `parcel` CLI (parcel publish, info, cat, pull, list, update, delete, comments, data). Do NOT use it for files that belong in the user's repository (write and commit those instead), for an answer that fits comfortably in chat, or for unrelated things named Parcel such as the parcel.js web bundler or package tracking."
---

# Publishing pages with parcel

Parcel turns an HTML page (plus any supporting files) into a URL that renders it in a sandboxed browser view. Parcels are **private by default**: only the signed-in owner can open them. Every publish is an immutable version; publishing again to the same parcel keeps the URL. The `parcel` CLI does everything below; `parcel <command> --help` is the authoritative flag reference.

## First: check the CLI and the login

```bash
parcel whoami        # exit 0 = signed in; exit 3 = not logged in
```

- **Exit 3 (not logged in):** ask the user to run `parcel login` in their own terminal (in Claude Code they can type `! parcel login`). It opens a browser and waits for them to approve a code, so **do not run it yourself**. Continue once `parcel whoami` succeeds.
- **`parcel: command not found`:** ask the user to install it (`brew install charliek/tap/parcel`, or the `parcel` package from the charliek apt repository), then run `parcel skill install` to refresh this skill.
- Never print or echo a token, and never put one on a command line. The CLI reads the saved login (or `PARCEL_TOKEN` from the environment) by itself.

## When to publish

Publish when the result is something to **look at** — layout, color, charts, interaction — or something long and structured the user will read, skim or forward. Ask first, or just answer in chat, when the answer is a few paragraphs, a code snippet, or a file that belongs in their project. Never publish something the user asked to keep local.

## Author the page

Default to **one self-contained HTML file**: all CSS in `<style>`, all JS in `<script>`, images as inline SVG or `data:` URIs. Start from this skeleton:

```html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Q3 latency report</title>
  <style>
    :root { color-scheme: light dark; --bg: #ffffff; --fg: #1a1a1a; --muted: #5f6368; --accent: #2563eb; }
    @media (prefers-color-scheme: dark) { :root { --bg: #111317; --fg: #e8eaed; --muted: #9aa0a6; --accent: #7aa2ff; } }
    body { margin: 0; background: var(--bg); color: var(--fg); font: 16px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }
    main { max-width: 960px; margin: 0 auto; padding: 24px 16px; }
    img, svg, canvas, table { max-width: 100%; }
  </style>
</head>
<body>
  <main>
    <h1>Q3 latency report</h1>
  </main>
</body>
</html>
```

Checklist — every page:

- `<meta charset="utf-8">`, the viewport meta, and a real `<title>` (it becomes the parcel's title unless you pass `--title`).
- Light **and** dark themes via `prefers-color-scheme`, with colors as CSS variables and an explicit `body` background.
- Works at phone width: a 16px side gutter, no horizontal page scroll, wide tables in an `overflow-x: auto` wrapper.
- No placeholder text, lorem ipsum or broken images; real data, labeled axes, readable contrast.

Use a **folder** instead when the page needs real assets (several pages, big data files, images, fonts): put the entry page at `index.html` and reference everything with **relative paths** (`assets/app.js`, `./data.json`). A leading `/` (`/assets/app.js`) breaks, because the parcel is served under a token prefix. A Markdown file (`.md`) also works as the entry page and is rendered into a styled page.

More design guidance, a chart example and multi-file layouts: [references/authoring.md](references/authoring.md).

## Publish

```bash
parcel publish report.html --title "Q3 latency report" --json
parcel publish ./site --title "Pricing prototype" --json     # a folder; index.html is the entry
```

Read `.parcel.url` (the link to give the user), `.parcel.id` and `.version.version` from the JSON. Then tell the user the URL, that it is private to their account, and what is in it. Write the files somewhere temporary (a scratch directory), not into the user's repo, unless they asked for them there.

- `--description "…"` sets a description; `--label "…"` notes what changed in this version.
- A folder publish skips dotfiles, dot-directories and `node_modules`, and skips symlinks with a warning. `--file PUBLISHED=LOCAL` adds a file from elsewhere (`--file data/q3.json=/tmp/out.json`).
- The CLI checks paths, file types and size limits before uploading; fix what it reports rather than retrying.

## Revise without changing the URL

When the user asks for changes to a parcel you published (or one they give you), publish a new version to it:

```bash
parcel publish report.html --parcel <id-or-url> --label "Fix axis labels" --json
parcel publish ./site --parcel <id> --json                       # folder: replaces all files
parcel publish --parcel <id> --file assets/data.json=out.json    # patch one file, keep the rest
parcel publish --parcel <id> --remove old.css                    # drop a file
```

- `--mode replace` (the default for a folder): the new version is exactly the files sent. `--mode merge` (the default for a single file or bare `--file`/`--remove`): files sent are added or replaced, others are kept.
- `--base-version N` fails with a conflict if someone published since version N — use it when the user may be editing too.
- Old versions stay viewable at `<url>/v/<n>`; `parcel versions <id>` lists them and `parcel restore <id> --version N` makes an old one current again.

## Act on feedback (comments)

People comment on a parcel in the web viewer's comment mode, usually by clicking an element of the page. When the user says there's feedback or there are comments on a parcel:

```bash
parcel comments list <ref>     # open threads; --status all includes resolved ones; --json for anchors
```

1. Read every thread. `#3 · v2 · index.html › "Revenue by region" · Sam · 2h ago` is comment 3, made on version 2, about the element on `index.html` whose text starts "Revenue by region" (a CSS selector is shown when the element had no text; `general` means the whole parcel). The body is indented below it, replies below that.
2. Make the changes in the source (`parcel pull <ref> ./work` if you don't have it), then publish a new version **to the same parcel** so the link stays the same: `parcel publish … --parcel <ref> --label "Address comments"`.
3. Resolve what you fixed, saying what changed: `parcel comments resolve <ref> 3 5 --reply "Fixed in v4: bars are blue, axis labelled"`. For a question or something you didn't change, `parcel comments reply <ref> N "…"` and leave it open.

`parcel info <ref>` shows the open-comment count. Commenting needs a login, even on public parcels.

## Read an existing parcel

```bash
parcel info <id-or-url> --json        # metadata, version history, file list (ParcelDetail)
parcel cat <id-or-url>                # the entry page's source, to stdout
parcel cat <id-or-url> assets/app.js  # one file
parcel pull <id-or-url> ./work        # every file, to edit locally and republish with --parcel
```

A URL ending in `/v/<n>` reads that version. Public parcels are readable without logging in.

## The sandbox

Parcels run with an **opaque origin** in a sandbox, on a separate content domain:

| Works | Does not work |
|---|---|
| Inline and external `<script>` (CDN scripts load, but inline when you can), ES modules, Web Workers | `IndexedDB` **throws** — wrap it in `try`/`catch`. `localStorage`, `sessionStorage` and `document.cookie` work but live **in memory**: they reset on every reload |
| `fetch("data.json")` and other relative URLs to the parcel's own files | Navigating the top window (`top.location`), reading the parent page |
| Forms, `alert`/`confirm`/`prompt`, downloads, `window.open` | Logins, payments or anything relying on cookies or the user's identity |
| Links with `target="_blank"` open in a new tab | Plain external links try to load inside the frame (many sites refuse) — give them `target="_blank" rel="noopener"` |

## Pages that remember (`window.parcel.data`)

Parcel adds a small helper script to every HTML page; it defines `window.parcel`. Use `window.parcel.data` for anything that must survive a reload (checklists, settings, form drafts, a dashboard's numbers): JSON items in named collections, kept on the server and shared by every version of the parcel.

```js
const data = window.parcel?.data;                    // undefined when opened as a local file
const prefs = (await data?.get("settings", "prefs")) ?? { theme: "auto" };
if (window.parcel?.canWrite) await data.update("settings", "prefs", { theme: "dark" });
data?.subscribe(render);                             // live: call your render() when the data changes
```

- **Only the owner's own viewing can write.** Visitors (anyone else on a public parcel) can read but not change it. Check `parcel.canWrite` and show a read-only view when it is `false`; writes would reject with status 403.
- **Agents can feed a page without republishing:** `parcel data set <ref> dashboard metrics @metrics.json` (or `parcel data update` to merge). A page that reads `parcel.data` and subscribes shows the new numbers live.
- Limits: 256 KiB per value, 10 MiB per parcel. Names: letters, digits, `_ . : -`. Don't name your own global `parcel`; it hides the helper.

Full API, a complete example and the CLI commands: [references/authoring.md](references/authoring.md), [references/cli.md](references/cli.md).

## Limits and file types

- **Sizes:** entry page 16 MiB, any other file 15 MiB, 64 MiB and 256 files per publish (512 files and 256 MiB per version). `parcel whoami` shows the storage quota.
- **Entry page:** `.html`, `.htm`, `.md`, `.markdown`.
- **Supporting files:** `css js mjs map wasm json geojson txt csv tsv xml yaml yml svg png jpg jpeg gif webp avif ico bmp woff woff2 ttf otf mp3 wav ogg m4a mp4 webm pdf glb gltf`. Anything else is rejected; convert it or leave it out.

## Visibility and safety

- New parcels are **private**. Use `--public` (anyone with the link, no sign-in) only when the user explicitly asks to share it publicly; `parcel update <id> --public` / `--private` changes it later.
- **Never publish secrets**: API keys, tokens, passwords, `.env` contents, private keys, connection strings, or personal data the user did not ask to share. Check data files and inline JSON before publishing a folder, and publish a narrow folder rather than a project root. The same goes for `parcel data set`: anyone who can view the parcel can read its data.
- `parcel delete <id> --yes` is permanent (every version). Only delete when the user asks, and confirm which parcel first; without a terminal the CLI refuses unless `--yes` is given.

## Other commands

| Task | Command |
|---|---|
| List the user's parcels | `parcel list [--query TEXT] [--limit N] [--json]` |
| Rename / re-describe / change visibility | `parcel update <ref> [--title …] [--description …] [--public\|--private]` |
| Open in the user's browser | `parcel open <ref>` |
| Comments | `parcel comments list\|add\|reply\|resolve\|reopen\|delete <ref> …` |
| A page's data | `parcel data list\|get\|set\|update\|delete\|clear <ref> …` |
| Show config (server, profile, token set?) | `parcel config show` |

`--json` prints the server's JSON response verbatim for `whoami`, `publish`, `list`, `info`, `update`, `versions`, `restore`, `comments list|add|reply` and `data list|get|set|update`. Exit codes: `0` ok, `1` error, `2` bad arguments, `3` not logged in or token rejected, `4` not found (a private parcel also looks like this when signed out). Errors print as `error: <message>` on stderr.

## References

- [references/authoring.md](references/authoring.md) — design checklist, a chart example, multi-file layouts, sandbox-safe patterns, the `window.parcel` API
- [references/cli.md](references/cli.md) — every command and flag (including `comments` and `data`), JSON shapes, config and environment
- [references/troubleshooting.md](references/troubleshooting.md) — exit codes, error messages and fixes (comments and data too), blank-page debugging

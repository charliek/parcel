# Authoring parcels

How to build pages that look right and work inside the parcel sandbox.

## Design checklist

Before publishing, check every item:

- **Head:** `<!doctype html>`, `<html lang="…">`, `<meta charset="utf-8">`, `<meta name="viewport" content="width=device-width, initial-scale=1">`, and a specific `<title>` (two to six words; it becomes the parcel title).
- **Themes:** set `color-scheme: light dark`, define every color as a CSS variable on `:root`, and override them under `@media (prefers-color-scheme: dark)`. Give `body` an explicit background and text color. Check charts, borders and code blocks in both themes.
- **Layout:** one readable column (`max-width` 720–1100px) centred with `margin: 0 auto`, a 16px side gutter, no horizontal scroll at 360px width. Wrap wide tables and code in `overflow-x: auto`. Use CSS grid or flexbox with `minmax()`/`flex-wrap` so card rows reflow on phones.
- **Type:** system font stack (`system-ui, -apple-system, "Segoe UI", sans-serif`), 16px base, line-height around 1.5, a clear heading hierarchy (one `h1`).
- **Content:** real data and real labels, no lorem ipsum, no "TODO". Put the conclusion first (a summary line or KPI row), details after.
- **Charts:** label axes and units, start bar axes at zero, use a legend only when there are several series, and keep colors distinguishable in both themes. Inline SVG is the most robust; `<canvas>` with a small inline script is fine.
- **Accessibility:** alt text on images, `aria-label` on icon-only buttons, text contrast at least 4.5:1, focus styles left on.
- **Self-contained:** inline CSS and JS; embed small images as inline SVG or `data:` URIs. External CDN scripts load, but each one is a dependency that can disappear or be blocked — inline the code when practical.
- **Safety:** no secrets, tokens or private data in the HTML, inline JSON or data files.

## Single-file template with a chart

```html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Weekly signups</title>
  <style>
    :root {
      color-scheme: light dark;
      --bg: #ffffff; --fg: #1a1a1a; --muted: #5f6368; --line: #e3e5e8; --bar: #2563eb;
    }
    @media (prefers-color-scheme: dark) {
      :root { --bg: #111317; --fg: #e8eaed; --muted: #9aa0a6; --line: #2a2e35; --bar: #7aa2ff; }
    }
    * { box-sizing: border-box; }
    body { margin: 0; background: var(--bg); color: var(--fg);
           font: 16px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }
    main { max-width: 880px; margin: 0 auto; padding: 24px 16px 48px; }
    .kpis { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 12px; }
    .kpi { border: 1px solid var(--line); border-radius: 10px; padding: 12px 14px; }
    .kpi b { display: block; font-size: 1.6rem; }
    .muted { color: var(--muted); }
    svg { width: 100%; height: auto; display: block; }
    /* SVG presentation attributes can't use var(); style the marks with CSS. */
    .bar { fill: var(--bar); }
    .axis-label { fill: var(--muted); font-size: 13px; }
    .scroll { overflow-x: auto; }
    table { border-collapse: collapse; width: 100%; }
    th, td { text-align: left; padding: 6px 8px; border-bottom: 1px solid var(--line); }
  </style>
</head>
<body>
<main>
  <h1>Weekly signups</h1>
  <p class="muted">Up 18% week over week; the Tuesday launch drove most of it.</p>
  <section class="kpis">
    <div class="kpi"><span class="muted">This week</span><b id="total">–</b></div>
    <div class="kpi"><span class="muted">Best day</span><b id="best">–</b></div>
  </section>
  <h2>By day</h2>
  <svg id="chart" viewBox="0 0 640 260" role="img" aria-label="Signups by day"></svg>
  <script>
    const data = [["Mon", 120], ["Tue", 410], ["Wed", 260], ["Thu", 220], ["Fri", 180]];
    const svg = document.getElementById("chart");
    const max = Math.max(...data.map(d => d[1]));
    const w = 640, h = 260, pad = 32, bw = (w - pad * 2) / data.length;
    const ns = "http://www.w3.org/2000/svg";
    data.forEach(([label, value], i) => {
      const bh = (h - pad * 2) * value / max;
      const rect = document.createElementNS(ns, "rect");
      Object.entries({ x: pad + i * bw + 8, y: h - pad - bh, width: bw - 16, height: bh,
                       rx: 4, class: "bar" }).forEach(([k, v]) => rect.setAttribute(k, v));
      const text = document.createElementNS(ns, "text");
      Object.entries({ x: pad + i * bw + bw / 2, y: h - 10, "text-anchor": "middle",
                       class: "axis-label" }).forEach(([k, v]) => text.setAttribute(k, v));
      text.textContent = label;
      svg.append(rect, text);
    });
    document.getElementById("total").textContent = data.reduce((s, d) => s + d[1], 0);
    document.getElementById("best").textContent = data.find(d => d[1] === max)[0];
  </script>
</main>
</body>
</html>
```

## Multi-file parcels

```
site/
├── index.html          entry page (or pass --entry)
├── about.html          other pages: link with href="about.html"
├── assets/
│   ├── app.js          <script type="module" src="assets/app.js">
│   └── style.css       <link rel="stylesheet" href="assets/style.css">
└── data/
    └── results.json    fetch("data/results.json")
```

- Every reference is **relative** (`assets/app.js`, `./data/x.json`, `../index.html` from a subfolder). Never start a path with `/`, and don't add a `<base>` tag.
- Publish with `parcel publish site --title "…" --json`. Dotfiles, dot-directories and `node_modules` are skipped; symlinks are skipped with a warning.
- To revise, publish the folder again with `--parcel <id>` (replaces every file), or send only what changed: `parcel publish --parcel <id> --file data/results.json=out/results.json`.
- Build tools: point the publish at the build output (`dist/`), and configure the bundler for relative asset URLs (Vite `base: "./"`, webpack `publicPath: "auto"` or `""`).

## Sandbox-safe patterns

The parcel's document has an opaque origin, where browser storage normally throws. Parcel's page helper (below) replaces `localStorage`, `sessionStorage` and `document.cookie` with in-memory stand-ins, so code written for a normal site keeps running, but **everything stored there is gone on reload**. Use them for state within one visit; use `window.parcel.data` for anything that must persist. `indexedDB` still throws: wrap it in `try`/`catch`.

- **Data files:** `const rows = await (await fetch("data/results.json")).json();` works (relative URL, same parcel version). Absolute URLs to other sites work only if those sites allow cross-origin requests from any origin.
- **Links:** give external links `target="_blank" rel="noopener"`. Links between the parcel's own pages can be plain relative links.
- **No top navigation:** `top.location = …`, `window.parent` access and `postMessage` handshakes with the viewer are not available.
- **No identity:** cookies don't exist, so a parcel can't sign anyone in. Don't build login forms or anything that asks for credentials.
- **Dialogs, forms, downloads, popups** work: `alert()`, `confirm()`, `<a download>`, `window.open()`.
- **Embedding other sites** in an `<iframe>` depends on that site's own framing policy; most block it.

## The page helper and `window.parcel`

Parcel adds one script to every HTML page it serves, `<script src="/_parcel/helper.<hash>.js"></script>`, right after `<head>`, so it runs before the page's own scripts. It:

- swaps the throwing storage APIs for in-memory stand-ins (above);
- defines `window.parcel` (below);
- drives the web viewer's comment mode: highlighting what the user hovers, reporting which element they click, and drawing comment pins. It only talks to the Parcel app that frames the page.

It is added when the page is served: `parcel cat` and `parcel pull` return the files exactly as published, without it. Don't define a global named `parcel` (`var parcel`, `let parcel`, `function parcel`, `window.parcel = …`) — yours wins and hides the helper's API. When the page is opened as a local file the helper isn't there, so guard with `window.parcel?.`.

| Member | Meaning |
|---|---|
| `parcel.id`, `parcel.version` | The parcel's id and the version being shown |
| `parcel.path` | This page's published path (`index.html`) |
| `parcel.canWrite` | `true` only while the parcel's owner views it signed in; `false` for everyone else (including the owner on a signed-out public link) |
| `await parcel.data.get(collection, key)` | The stored value, or `undefined` |
| `await parcel.data.list(collection)` | Every item, by key: `[{collection, key, value, size_bytes, updated_at}]` |
| `await parcel.data.set(collection, key, value)` | Create or replace an item with any JSON value; resolves to it |
| `await parcel.data.update(collection, key, patch)` | Merge an object into an item (RFC 7386: nested objects merge, `null` removes a field); creates it if missing |
| `await parcel.data.delete(collection, key)` | Remove one item |
| `await parcel.data.clear(collection)` | Remove every item in a collection |
| `await parcel.data.collections()` | `[{name, count, usage_bytes, updated_at}]` |
| `parcel.data.subscribe(callback)` | Calls `callback({revision})` whenever the parcel's data changes — from another tab, the CLI or the page's own writes (checked every few seconds while the page is visible). Returns an unsubscribe function |

Data is shared by every version of the parcel and deleted with it. Anyone who can view the parcel can read it; only the owner writes. A failed call rejects with an `Error` whose `status` is the HTTP status: 403 when writing without `canWrite`, 413 over a limit, 400 for a bad name. Names (collections and keys) are 1–128 of `A–Z a–z 0–9 _ . : -`, starting with a letter or digit. Limits: 256 KiB per value, 10 MiB per parcel (each item counts its JSON plus 128 bytes).

### Example: a checklist that persists

The owner ticks items and they stay ticked; anyone else who can open the parcel (shared with, internal or public) sees the same state read-only.

```html
<ul id="todo"></ul>
<p id="readonly" class="muted" hidden>Read-only: only the owner can tick items.</p>
<script>
  const tasks = ["Draft the outline", "Collect Q3 numbers", "Review with Sam"];
  const data = window.parcel?.data;                 // undefined when opened as a local file
  const canWrite = Boolean(window.parcel?.canWrite);
  document.getElementById("readonly").hidden = canWrite || !data;

  async function render() {
    const done = (await data?.get("checklist", "done")) ?? {};
    document.getElementById("todo").replaceChildren(...tasks.map((task, i) => {
      const box = Object.assign(document.createElement("input"),
                                { type: "checkbox", checked: Boolean(done[i]), disabled: !canWrite });
      box.addEventListener("change", () =>
        data.update("checklist", "done", { [i]: box.checked })
            .catch((err) => { box.checked = !box.checked; alert(err.message); }));
      const label = document.createElement("label");
      label.append(box, " ", task);
      const li = document.createElement("li");
      li.append(label);
      return li;
    }));
  }
  render();
  data?.subscribe(render);                          // another tab or `parcel data set` changed it
</script>
```

### Example: a dashboard an agent keeps current

The page renders whatever is in `dashboard/metrics`; an agent refreshes it with the CLI, no republish needed, and open viewers update live:

```js
async function render() {
  const metrics = await window.parcel?.data.get("dashboard", "metrics");
  if (metrics) drawCharts(metrics);           // your rendering code
}
render();
window.parcel?.data.subscribe(render);
```

```bash
parcel data set <ref> dashboard metrics @metrics.json        # replace
parcel data update <ref> dashboard metrics '{"p99_ms": 412}'  # merge one field
```

## Markdown entry pages

A `.md` or `.markdown` entry is rendered server-side into a styled HTML page (headings, lists, tables, code blocks, links). Use it for prose-heavy documents where layout doesn't matter; switch to HTML for charts, custom layout or interaction. Supporting files (images) can sit next to it and be referenced relatively.

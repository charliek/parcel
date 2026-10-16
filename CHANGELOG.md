# Changelog

## Unreleased

First cut of Parcel, built and verified locally.

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

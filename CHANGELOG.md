# Changelog

## Unreleased

First cut of Parcel, built and verified locally.

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

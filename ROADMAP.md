# Roadmap

## Next
- **MCP server** at `/mcp`: stateless streamable HTTP, RFC 9728 metadata,
  a separate verifier with `aud=https://parcel…/mcp`, scopes
  `parcel:read`/`parcel:write`, connector clients in slauth (the slaudio
  design, ported to Rust). Tools: publish, list, read, update, delete.
- Publish the repo to GitHub; turn on CI, the CLI release (brew/apt) and the
  image release + deploy workflows; first staging deploy (Neon, bucket,
  secrets, domain mappings — see deploy/README.md).
- Rate limiting for the unauthenticated device-start endpoint, publishes,
  comments and data writes.

## Later
- Per-parcel origins (wildcard subdomain + certificate) so parcels can use a
  real, persistent `localStorage`/`IndexedDB` (today's stand-ins reset on
  reload).
- Sharing extras: an editor role (page data and new versions), sharing with
  a provider group (Okta/Keycloak `groups` claim), invite emails and "shared
  with you" notifications, a browsable list of internal parcels.
- An admin page: users, sign-ins, disabling people (today: SQL), and SCIM
  deprovisioning from Okta/Keycloak.
- Visitor writes to page data (opt-in "collect responses" for polls and
  sign-up sheets, with rate limits), and per-viewer private data.
- Comment notifications (email/webhook) and `parcel comments --watch` so an
  agent can wait for feedback.
- Zip download of a version; thumbnails in the dashboard.
- Accept slauth PATs (`slp_live_…`) via `/api/v1/pat/validate` alongside `pcl_` tokens.

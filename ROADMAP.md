# Roadmap

## Next
- **MCP server** at `/mcp`: stateless streamable HTTP, RFC 9728 metadata,
  a separate verifier with `aud=https://parcel…/mcp`, scopes
  `parcel:read`/`parcel:write`, connector clients in slauth (the slaudio
  design, ported to Rust). Tools: publish, list, read, update, delete.
- Publish the repo to GitHub; turn on CI, the CLI release (brew/apt) and the
  image release + deploy workflows; first staging deploy (Neon, bucket,
  secrets, domain mappings — see deploy/README.md).
- Rate limiting for the unauthenticated device-start endpoint and publishes.

## Later
- Per-parcel origins (wildcard subdomain + certificate) so parcels can use
  `localStorage`/`IndexedDB` safely.
- Sharing with named users; comments; per-parcel shared data.
- Zip download of a version; thumbnails in the dashboard.
- Accept slauth PATs (`slp_live_…`) via `/api/v1/pat/validate` alongside `pcl_` tokens.

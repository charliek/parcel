# Roadmap

## Next
- A staging environment (Neon branch, bucket, secrets, domain mappings — see
  deploy/README.md) and the CLI release (brew/apt).
- slauth access tokens carry no `email_verified`; add it (from Kratos'
  verifiable addresses) so parcel can stop trusting slauth emails
  (`AUTH_SLAUTH_TRUST_EMAIL=false`) before another provider joins.
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

# Local Keycloak (a second sign-in provider)

For trying Parcel with more than one OIDC provider. Not used by the test
suites, which mock providers.

```bash
docker compose --profile keycloak up -d keycloak   # http://localhost:4448 (admin / localdev)
```

The `parcel` realm is imported from `parcel-realm.json` on first start: a
confidential client `parcel` (PKCE required) and these users (password =
username):

| User | Email | Why |
|---|---|---|
| alice, bruno | `@eu.example.com`, verified | sign in normally |
| mallory | `@eu.example.com`, **not** verified | refused (`error=unverified`) |
| olga | `@elsewhere.example`, verified | refused when the provider is limited to `eu.example.com` (`error=domain`) |

Then, alongside slauth, in `.env`:

```bash
AUTH_PROVIDERS=slauth,keycloak
AUTH_SLAUTH_LABEL="Stride Labs"
AUTH_KEYCLOAK_ISSUER=http://localhost:4448/realms/parcel
AUTH_KEYCLOAK_CLIENT_ID=parcel
AUTH_KEYCLOAK_CLIENT_SECRET=parcel-local-keycloak-secret
AUTH_KEYCLOAK_LABEL="EU office (Keycloak)"
AUTH_KEYCLOAK_ALLOWED_DOMAINS=eu.example.com
```

(`http://` issuers are accepted only with `APP_ENV=development`.) Realm
changes in the JSON apply only to a fresh container:
`docker compose rm -sf keycloak` first.

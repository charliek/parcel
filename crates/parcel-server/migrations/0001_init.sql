-- Parcel schema, v1.
--
-- users           one row per slauth identity (keyed by the token `sub`)
-- parcels         the thing a URL points at; owns an ordered set of versions
-- parcel_versions immutable snapshots; a publish appends one
-- version_files   path -> blob for each version
-- blobs           content-addressed bytes in the blob store, shared across
--                 versions (and users) and garbage-collected when unreferenced
-- api_tokens      `pcl_…` bearer tokens (CLI + agents); only the SHA-256 is kept
-- device_authorizations  pending CLI logins (RFC 8628-style device flow)

CREATE TABLE users (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- slauth identity UUID (OIDC `sub`), or `dev:<email>` under AUTH_MODE=dev.
    sub          TEXT NOT NULL UNIQUE,
    email        TEXT NOT NULL,
    name         TEXT NOT NULL DEFAULT '',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE parcels (
    id              TEXT PRIMARY KEY,
    owner_id        UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title           TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    visibility      TEXT NOT NULL DEFAULT 'private'
                    CHECK (visibility IN ('private', 'public')),
    current_version INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX parcels_owner_updated_idx ON parcels (owner_id, updated_at DESC);

CREATE TABLE parcel_versions (
    parcel_id   TEXT NOT NULL REFERENCES parcels (id) ON DELETE CASCADE,
    version     INTEGER NOT NULL CHECK (version > 0),
    entry       TEXT NOT NULL,
    label       TEXT,
    file_count  INTEGER NOT NULL,
    total_bytes BIGINT NOT NULL,
    -- 'api' (CLI/agents/web upload) or 'restore'.
    created_via TEXT NOT NULL DEFAULT 'api',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (parcel_id, version)
);

CREATE TABLE blobs (
    sha256     TEXT PRIMARY KEY,
    size_bytes BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Bumped whenever a publish references the blob. GC only collects blobs
    -- that are unreferenced AND untouched for a grace period, which is what
    -- keeps a publish that is mid-flight (bytes written, rows not yet
    -- committed) from losing its objects.
    touched_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE version_files (
    parcel_id    TEXT NOT NULL,
    version      INTEGER NOT NULL,
    path         TEXT NOT NULL,
    blob_sha256  TEXT NOT NULL REFERENCES blobs (sha256),
    size_bytes   BIGINT NOT NULL,
    content_type TEXT NOT NULL,
    PRIMARY KEY (parcel_id, version, path),
    FOREIGN KEY (parcel_id, version)
        REFERENCES parcel_versions (parcel_id, version) ON DELETE CASCADE
);

CREATE INDEX version_files_blob_idx ON version_files (blob_sha256);

CREATE TABLE api_tokens (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id        UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name           TEXT NOT NULL,
    token_hash     TEXT NOT NULL UNIQUE,
    prefix_display TEXT NOT NULL,
    -- 'web' (Tokens page) or 'device' (CLI login).
    created_via    TEXT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at   TIMESTAMPTZ,
    expires_at     TIMESTAMPTZ,
    revoked_at     TIMESTAMPTZ
);

CREATE INDEX api_tokens_user_idx ON api_tokens (user_id, created_at DESC);

CREATE TABLE device_authorizations (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    device_code_hash TEXT NOT NULL UNIQUE,
    user_code        TEXT NOT NULL UNIQUE,
    client_name      TEXT NOT NULL,
    -- pending -> approved | denied; approved -> redeemed once the CLI collects
    -- its token (which is minted at that moment, never stored raw).
    status           TEXT NOT NULL DEFAULT 'pending'
                     CHECK (status IN ('pending', 'approved', 'denied', 'redeemed')),
    user_id          UUID REFERENCES users (id) ON DELETE CASCADE,
    interval_secs    INTEGER NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at       TIMESTAMPTZ NOT NULL,
    last_polled_at   TIMESTAMPTZ
);

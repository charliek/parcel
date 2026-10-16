-- Comments, per-parcel data, and the counters the viewer polls.
--
-- comments     feedback pinned to a spot on a parcel's page (or general), with
--              one level of replies; numbered per parcel (#1, #2, …) so the
--              CLI and agents can refer to them
-- parcel_data  small JSON documents a parcel's page (or its owner's agent)
--              keeps between visits; shared by every version of the parcel

ALTER TABLE parcels
    -- Last comment number handed out; bumped under the parcel row lock.
    ADD COLUMN comment_seq  INTEGER NOT NULL DEFAULT 0,
    -- Bumped on every comment change so the viewer can poll one cheap row.
    ADD COLUMN comments_rev BIGINT  NOT NULL DEFAULT 0,
    -- Bumped on every data change, for the page helper's change events.
    ADD COLUMN data_rev     BIGINT  NOT NULL DEFAULT 0;

CREATE TABLE comments (
    parcel_id     TEXT    NOT NULL REFERENCES parcels (id) ON DELETE CASCADE,
    number        INTEGER NOT NULL CHECK (number > 0),
    -- The version the commenter was looking at.
    version       INTEGER NOT NULL,
    -- Set on replies; replies never nest further and never carry an anchor.
    parent_number INTEGER,
    author_id     UUID    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- 'web' (the viewer) or 'cli' (a `pcl_` token: the CLI or an agent).
    via           TEXT    NOT NULL CHECK (via IN ('web', 'cli')),
    body          TEXT    NOT NULL,
    -- Where on the page: {path, selector?, text?, x?, y?}. NULL = general.
    anchor        JSONB,
    resolved_at   TIMESTAMPTZ,
    resolved_by   UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (parcel_id, number),
    FOREIGN KEY (parcel_id, version)
        REFERENCES parcel_versions (parcel_id, version) ON DELETE CASCADE,
    FOREIGN KEY (parcel_id, parent_number)
        REFERENCES comments (parcel_id, number) ON DELETE CASCADE,
    CHECK (parent_number IS NULL OR (anchor IS NULL AND resolved_at IS NULL))
);

CREATE INDEX comments_open_threads_idx ON comments (parcel_id)
    WHERE parent_number IS NULL AND resolved_at IS NULL;

CREATE TABLE parcel_data (
    parcel_id  TEXT   NOT NULL REFERENCES parcels (id) ON DELETE CASCADE,
    collection TEXT   NOT NULL,
    key        TEXT   NOT NULL,
    value      JSONB  NOT NULL,
    -- Length of the value as compact JSON text.
    size_bytes BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (parcel_id, collection, key)
);

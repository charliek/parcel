-- Tags, archive and full-text search.
--
-- Additive and rollout-safe: during a Cloud Run rollover the previous
-- revision keeps inserting and versioning parcels, so every new column has a
-- default or is nullable, and the search columns are maintained by triggers
-- (which also run for the old revision's writes).
--
-- tags            free-form labels, normalized and sorted by bytes by the
--                 app (≤ 20); SQL that rewrites them sorts COLLATE "C" too
-- archived_at     set while the owner has the parcel archived; any new
--                 current version clears it (trigger below)
-- search_text     plain text of the current version's entry page (≤ 128 KiB),
--                 NULL until extracted (after the publish commits, or by the
--                 backfill loop); '' when the page has no text
-- search_attempts failed extractions since the last version change; the
--                 backfill gives up on a parcel after a few
-- search_vec      title (A) + description and tags (B) + page text (D), all
--                 with the `english` configuration the query uses (tags
--                 too: indexed `simple`, a tag like `heliotrope` would never
--                 match the stemmed query `heliotrop`)

ALTER TABLE parcels
    ADD COLUMN tags            TEXT[]   NOT NULL DEFAULT '{}',
    ADD COLUMN archived_at     TIMESTAMPTZ,
    ADD COLUMN search_text     TEXT,
    ADD COLUMN search_vec      TSVECTOR,
    ADD COLUMN search_attempts SMALLINT NOT NULL DEFAULT 0;

-- The one definition of a parcel's search document, used by the trigger and
-- the population below. A function rather than a generated column:
-- array_to_string isn't immutable.
CREATE FUNCTION parcel_search_document(title TEXT, description TEXT, tags TEXT[], body TEXT)
RETURNS TSVECTOR LANGUAGE sql STABLE AS $$
    SELECT setweight(to_tsvector('english', coalesce(title, '')), 'A')
        || setweight(to_tsvector('english', coalesce(description, '')), 'B')
        || setweight(to_tsvector('english', array_to_string(coalesce(tags, '{}'), ' ')), 'B')
        || setweight(to_tsvector('english', coalesce(body, '')), 'D')
$$;

-- What a new current version means, whoever wrote it (this revision, or an
-- older one that knows nothing of search or archiving): the stored page text
-- is stale, and the parcel is live again, so it un-archives. Kept in the
-- database so an old revision's publish or restore during a rollover can't
-- leave the new version archived.
CREATE FUNCTION parcels_version_changed() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    NEW.search_text := NULL;
    NEW.search_attempts := 0;
    NEW.archived_at := NULL;
    RETURN NEW;
END
$$;

CREATE FUNCTION parcels_search_vec() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    NEW.search_vec := parcel_search_document(NEW.title, NEW.description, NEW.tags, NEW.search_text);
    RETURN NEW;
END
$$;

-- Postgres runs BEFORE triggers in name order, so `_a_version_changed`
-- clears the text before `_b_vec` reads it. `_b_vec` lists current_version too: an
-- `UPDATE … OF` trigger fires only for columns the statement itself sets,
-- not for ones an earlier trigger changed, so without it a plain version
-- bump would clear search_text but keep the old page's words in search_vec.
-- Comment and data revision bumps set none of these columns and skip both.
CREATE TRIGGER parcels_a_version_changed
    BEFORE UPDATE OF current_version ON parcels
    FOR EACH ROW
    WHEN (OLD.current_version IS DISTINCT FROM NEW.current_version)
    EXECUTE FUNCTION parcels_version_changed();

CREATE TRIGGER parcels_search_b_vec
    BEFORE INSERT OR UPDATE OF title, description, tags, search_text, current_version ON parcels
    FOR EACH ROW
    EXECUTE FUNCTION parcels_search_vec();

-- Existing rows: the triggers don't fire for rows nobody touches. Metadata
-- is searchable at once; page text follows from the backfill.
UPDATE parcels SET search_vec = parcel_search_document(title, description, tags, search_text);

ALTER TABLE parcels ALTER COLUMN search_vec SET NOT NULL;

CREATE INDEX parcels_tags_idx ON parcels USING GIN (tags);
CREATE INDEX parcels_search_idx ON parcels USING GIN (search_vec);
-- What the backfill loop scans for.
CREATE INDEX parcels_search_pending_idx ON parcels (id) WHERE search_text IS NULL;

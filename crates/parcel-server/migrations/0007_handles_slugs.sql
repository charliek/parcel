-- Handles, slugs, slug redirects and version sources.
--
-- Additive and rollout-safe, like 0006: the previous revision keeps
-- inserting users and parcels during a rollover, so `users` gains no column
-- (handles live in their own table, allocated lazily for a user an old
-- revision created) and every new parcel column is nullable.
--
-- user_handles           every handle a user has had: the current one
--                        (`is_current`) and the ones they renamed away
--                        from, which keep resolving to them. One primary
--                        key over both, so a retired handle can never be
--                        claimed by anyone else.
-- parcels.slug           a parcel's name under its owner's handle
--                        (`/@<handle>/<slug>`), unique per owner
-- parcel_slug_redirects  slugs a parcel had before a rename or clear; they
--                        resolve to it until another of the owner's parcels
--                        takes the name (which deletes the row)
-- parcel_versions.source where a version was published from (repo, branch,
--                        commit, host, client); the owner's only

CREATE TABLE user_handles (
    handle     TEXT PRIMARY KEY,
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    is_current BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX user_handles_current_idx ON user_handles (user_id) WHERE is_current;

-- The handle an email's local part suggests: lowercased (ASCII), `+suffix`
-- dropped, `.` and `_` turned into `-`, everything else but [a-z0-9-]
-- dropped, runs of `-` collapsed, `-` trimmed from both ends, `user` when
-- nothing is left, at most 32 characters (and no trailing `-` after the cut).
CREATE FUNCTION parcel_handle_base(email TEXT) RETURNS TEXT
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    h TEXT := email;
BEGIN
    -- The local part: everything before the last '@'.
    IF position('@' IN h) > 0 THEN
        h := left(h, length(h) - position('@' IN reverse(h)));
    END IF;
    -- ASCII only, and spelled out rather than ranges, so neither lower()
    -- nor a regex range depends on the database's locale.
    h := translate(h, 'ABCDEFGHIJKLMNOPQRSTUVWXYZ', 'abcdefghijklmnopqrstuvwxyz');
    h := split_part(h, '+', 1);
    h := translate(h, '._', '--');
    h := regexp_replace(h, '[^abcdefghijklmnopqrstuvwxyz0123456789-]', '', 'g');
    h := regexp_replace(h, '-+', '-', 'g');
    h := trim(BOTH '-' FROM h);
    h := rtrim(left(h, 32), '-');
    RETURN coalesce(nullif(h, ''), 'user');
END
$$;

-- The user's current handle, allocating one first if they have none: the
-- base from their email, else `<base>-2`, `<base>-3`, … (the base cut so the
-- whole fits in 32 characters). Each candidate is an `INSERT … ON CONFLICT DO
-- NOTHING`, so a taken one (by anyone, current or retired) costs no aborted
-- transaction. Checking for a current handle on every round makes concurrent
-- calls for one user converge on the same handle: a racing insert of theirs
-- conflicts on the one-current-handle index and the next round returns it.
-- The one implementation, used by this migration's backfill, sign-in, and the
-- lazy path for users an older revision created.
CREATE FUNCTION parcel_allocate_handle(p_user_id UUID, p_email TEXT) RETURNS TEXT
LANGUAGE plpgsql VOLATILE AS $$
DECLARE
    base      TEXT := parcel_handle_base(p_email);
    candidate TEXT;
    suffix    TEXT;
    n         INTEGER := 1;
    got       TEXT;
BEGIN
    LOOP
        SELECT handle INTO got FROM user_handles WHERE user_id = p_user_id AND is_current;
        IF got IS NOT NULL THEN
            RETURN got;
        END IF;
        IF n = 1 THEN
            candidate := base;
        ELSE
            suffix := '-' || n;
            candidate := rtrim(left(base, 32 - length(suffix)), '-') || suffix;
        END IF;
        INSERT INTO user_handles (handle, user_id, is_current)
        VALUES (candidate, p_user_id, true)
        ON CONFLICT DO NOTHING
        RETURNING handle INTO got;
        IF got IS NOT NULL THEN
            RETURN got;
        END IF;
        n := n + 1;
    END LOOP;
END
$$;

ALTER TABLE parcels ADD COLUMN slug TEXT;
CREATE UNIQUE INDEX parcels_owner_slug_idx ON parcels (owner_id, slug) WHERE slug IS NOT NULL;

CREATE TABLE parcel_slug_redirects (
    owner_id   UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    slug       TEXT NOT NULL,
    parcel_id  TEXT NOT NULL REFERENCES parcels (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (owner_id, slug)
);

CREATE INDEX parcel_slug_redirects_parcel_idx ON parcel_slug_redirects (parcel_id);

ALTER TABLE parcel_versions ADD COLUMN source JSONB;

-- Every existing user gets a handle, the earliest first (so the oldest
-- account keeps the plain name). A loop rather than `SELECT f(id) … ORDER BY`,
-- so the order is spelled out rather than left to when the planner
-- evaluates a volatile SELECT list.
DO $$
DECLARE
    u RECORD;
BEGIN
    FOR u IN SELECT id, email FROM users ORDER BY created_at, id LOOP
        PERFORM parcel_allocate_handle(u.id, u.email);
    END LOOP;
END
$$;

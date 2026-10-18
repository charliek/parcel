-- users.sub predates sign-in providers: 0003 copied it into user_identities
-- and kept the column (nullable, unwritten) only so a server from before 0003
-- kept working while that deploy rolled over. No running revision reads it
-- since, so it goes, with its UNIQUE constraint.
--
-- Refuse rather than lose a sign-in: every user that still has a legacy sub
-- must have an identity to sign in through.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM users u
        WHERE u.sub IS NOT NULL
          AND NOT EXISTS (SELECT 1 FROM user_identities i WHERE i.user_id = u.id)
    ) THEN
        RAISE EXCEPTION 'users with a legacy sub but no user_identities row; not dropping users.sub';
    END IF;
END
$$;

ALTER TABLE users DROP COLUMN sub;

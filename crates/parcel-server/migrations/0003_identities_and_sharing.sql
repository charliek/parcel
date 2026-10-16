-- v0.3: sign-in through several OIDC providers, and sharing.
--
-- user_identities  (provider, subject) -> user; a person signing in through
--                  two providers with the same email is one user
-- parcel_shares    (parcel, email, role); matched against the signed-in
--                  user's email at request time, so no pending-invite rows

CREATE TABLE user_identities (
    -- A configured provider id (`AUTH_PROVIDERS`), or `dev` under AUTH_MODE=dev.
    provider      TEXT NOT NULL,
    -- The provider's `sub`: unique only within that provider.
    subject       TEXT NOT NULL,
    user_id       UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    email         TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_login_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, subject)
);

CREATE INDEX user_identities_user_idx ON user_identities (user_id);

INSERT INTO user_identities (provider, subject, user_id, email, created_at, last_login_at)
SELECT CASE WHEN sub LIKE 'dev:%' THEN 'dev' ELSE 'slauth' END,
       CASE WHEN sub LIKE 'dev:%' THEN substr(sub, 5) ELSE sub END,
       id, email, created_at, last_seen_at
FROM users;

-- Kept (nullable, no longer written) so a server from before this migration
-- keeps working while a deploy rolls over; a later migration drops it.
ALTER TABLE users ALTER COLUMN sub DROP NOT NULL;
-- Bumped by "sign out everywhere": sessions carry the epoch they were
-- issued under and stop working when it moves.
ALTER TABLE users ADD COLUMN session_epoch INTEGER NOT NULL DEFAULT 0;
-- Last interactive sign-in. API tokens stop working when it is older than
-- PARCEL_TOKEN_LOGIN_MAX_AGE_DAYS, so offboarding at the provider reaches them.
ALTER TABLE users ADD COLUMN last_login_at TIMESTAMPTZ;
UPDATE users SET last_login_at = last_seen_at;
ALTER TABLE users ALTER COLUMN last_login_at SET NOT NULL;
ALTER TABLE users ALTER COLUMN last_login_at SET DEFAULT now();
-- Set by an operator (no UI yet); blocks sessions and tokens at once.
ALTER TABLE users ADD COLUMN disabled_at TIMESTAMPTZ;

CREATE INDEX users_email_idx ON users (lower(email));

ALTER TABLE parcels DROP CONSTRAINT parcels_visibility_check;
ALTER TABLE parcels ADD CONSTRAINT parcels_visibility_check
    CHECK (visibility IN ('private', 'internal', 'public'));

CREATE TABLE parcel_shares (
    parcel_id  TEXT NOT NULL REFERENCES parcels (id) ON DELETE CASCADE,
    -- Lowercased.
    email      TEXT NOT NULL,
    role       TEXT NOT NULL CHECK (role IN ('viewer', 'commenter')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (parcel_id, email)
);

CREATE INDEX parcel_shares_email_idx ON parcel_shares (email);

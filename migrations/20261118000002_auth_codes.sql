-- One-time codes sent by email (second sign-in step and password reset), and
-- browsers that completed the second step and may skip it for a while.

CREATE TABLE auth_codes (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    purpose     TEXT        NOT NULL CHECK (purpose IN ('login', 'password_reset')),
    -- Keyed SHA-256 of the code; the code itself is never stored.
    code_hash   TEXT        NOT NULL,
    -- Wrong guesses against this code; it stops working at the limit.
    attempts    INTEGER     NOT NULL DEFAULT 0,
    expires_at  TIMESTAMPTZ NOT NULL,
    -- Set when the code is used or superseded; a code works at most once.
    consumed_at TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX auth_codes_user_purpose_idx ON auth_codes (user_id, purpose, created_at DESC);
CREATE INDEX auth_codes_expires_at_idx ON auth_codes (expires_at);

CREATE TABLE trusted_devices (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- SHA-256 of the random token held in the browser's device cookie.
    token_hash   TEXT        NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL
);
CREATE UNIQUE INDEX trusted_devices_token_hash_key ON trusted_devices (token_hash);
CREATE INDEX trusted_devices_user_idx ON trusted_devices (user_id);
CREATE INDEX trusted_devices_expires_at_idx ON trusted_devices (expires_at);

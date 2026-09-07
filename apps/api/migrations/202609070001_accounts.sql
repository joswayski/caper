-- Internal references stay bigint; public IDs are random, immutable NanoIDs.
CREATE TABLE users (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    public_id text COLLATE "C" NOT NULL UNIQUE
        CHECK (public_id ~ '^[A-Za-z0-9_-]{21}$'),
    email text COLLATE "C" NOT NULL UNIQUE
        CHECK (email = lower(btrim(email)) AND char_length(email) BETWEEN 3 AND 254
               AND email ~ '^[^[:space:]@]+@[^[:space:]@]+$'),
    email_verified_at timestamptz NOT NULL DEFAULT now(),
    -- NULL until onboarding; both profile fields are set together.
    username text COLLATE "C" UNIQUE
        CHECK (username ~ '^[a-z0-9_]{3,32}$'),
    display_name text CHECK (char_length(btrim(display_name)) BETWEEN 1 AND 64),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((username IS NULL) = (display_name IS NULL))
);

CREATE TABLE sessions (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id bigint NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- SHA-256 of a cryptographically random bearer token, never the token itself.
    token_hash bytea NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    CHECK (expires_at > created_at)
);
CREATE INDEX sessions_user_id_idx ON sessions(user_id);
CREATE INDEX sessions_expires_at_idx ON sessions(expires_at);

-- One current challenge per normalized email, including not-yet-registered users.
-- Retain spent/locked challenges until expiry so resend cooldowns survive use.
CREATE TABLE login_challenges (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    public_id text COLLATE "C" NOT NULL UNIQUE
        CHECK (public_id ~ '^[A-Za-z0-9_-]{21}$'),
    email text COLLATE "C" NOT NULL UNIQUE
        CHECK (email = lower(btrim(email)) AND char_length(email) BETWEEN 3 AND 254
               AND email ~ '^[^[:space:]@]+@[^[:space:]@]+$'),
    -- HMAC-SHA-256 with a server-only key, bound to email + public_id + code.
    -- A plain hash of a six-digit code is not safe against offline guessing.
    code_hash bytea NOT NULL CHECK (octet_length(code_hash) = 32),
    attempts smallint NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 5),
    consumed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    CHECK (expires_at > created_at)
);
CREATE INDEX login_challenges_expires_at_idx ON login_challenges(expires_at);

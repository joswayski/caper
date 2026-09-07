-- Internal references stay bigint; public IDs are random, immutable NanoIDs.
CREATE TABLE users (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    public_id text COLLATE "C" NOT NULL UNIQUE
        CHECK (public_id ~ '^[A-Za-z0-9_-]{21}$'),
    workos_user_id text COLLATE "C" NOT NULL UNIQUE
        CHECK (workos_user_id ~ '^user_[A-Za-z0-9]+$'),
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

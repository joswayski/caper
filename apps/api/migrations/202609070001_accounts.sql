-- Internal references stay bigint; public IDs are random, immutable NanoIDs.
CREATE TABLE users (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    public_id text NOT NULL UNIQUE,
    workos_user_id text NOT NULL UNIQUE,
    email text NOT NULL UNIQUE,
    email_verified_at timestamptz NOT NULL DEFAULT now(),
    -- NULL until onboarding; both profile fields are set together.
    username text UNIQUE,
    display_name text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

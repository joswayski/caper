-- Bigint for internal references; username is the globally unique public identifier.
CREATE TABLE users (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    workos_user_id text NOT NULL UNIQUE,
    email text NOT NULL UNIQUE,
    email_verified_at timestamptz NOT NULL DEFAULT now(),
    -- NULL until onboarding; both profile fields are set together.
    username text UNIQUE,
    display_name text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

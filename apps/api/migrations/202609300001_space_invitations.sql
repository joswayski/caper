-- Pending invitations never count as membership or grant channel access.
-- Retain one row per pair for decline/removal cooldowns, not an unbounded event log.
CREATE TABLE space_invitations (
    space_id bigint NOT NULL REFERENCES spaces (id),
    user_id bigint NOT NULL REFERENCES users (id),
    status text NOT NULL CHECK (status IN ('pending', 'accepted', 'declined', 'revoked')),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (space_id, user_id)
);
CREATE INDEX space_invitations_user_id ON space_invitations (user_id);

-- Shared across API replicas, including failed lookups and duplicate attempts.
CREATE TABLE space_invite_limits (
    user_id bigint PRIMARY KEY REFERENCES users (id),
    window_start timestamptz NOT NULL DEFAULT now(),
    attempts integer NOT NULL DEFAULT 1
);

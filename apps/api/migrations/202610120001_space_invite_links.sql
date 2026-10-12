-- Shareable space invite links, modeled on Discord invites: anyone holding an
-- unexpired, unrevoked, unexhausted code may join the space. Links are retained
-- after revocation/expiry (revoked_at), never deleted.
CREATE TABLE space_invite_links (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    code text NOT NULL UNIQUE,
    space_id bigint NOT NULL REFERENCES spaces (id),
    -- Optional landing channel; joins fall back to the starter channel when it
    -- is deleted or made private after the link was created.
    channel_id bigint REFERENCES channels (id),
    created_by bigint NOT NULL REFERENCES users (id),
    max_uses integer CHECK (max_uses IS NULL OR max_uses > 0),
    uses integer NOT NULL DEFAULT 0 CHECK (uses >= 0),
    expires_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz
);
CREATE INDEX space_invite_links_space_id ON space_invite_links (space_id);

-- One row per join through a link, so owners can audit how members arrived.
CREATE TABLE space_invite_link_uses (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    link_id bigint NOT NULL REFERENCES space_invite_links (id),
    user_id bigint NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX space_invite_link_uses_link_id ON space_invite_link_uses (link_id);

-- Owner removals are distinct from leaving: a removed member cannot rejoin
-- through a public link until the owner invites them again by username.
ALTER TABLE space_invitations DROP CONSTRAINT space_invitations_status_check;
ALTER TABLE space_invitations ADD CONSTRAINT space_invitations_status_check
    CHECK (status IN ('pending', 'accepted', 'declined', 'revoked', 'removed'));

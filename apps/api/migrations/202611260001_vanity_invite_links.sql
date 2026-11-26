-- Vanity invite links (`/invite/<name>`): an owner-chosen, lowercase code that
-- never expires or runs out. A space has at most one live vanity link. Released
-- names are revoked, never deleted, so the row keeps the name reserved for the
-- space that used it: `code` stays unique, and a shared vanity link can never
-- start pointing at another space.
ALTER TABLE space_invite_links ADD COLUMN vanity boolean NOT NULL DEFAULT false;
CREATE UNIQUE INDEX space_invite_links_live_vanity ON space_invite_links (space_id)
    WHERE vanity AND revoked_at IS NULL;

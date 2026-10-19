-- Starting a DM looks an account up by username and can answer "not found" or
-- "not accepting". Charge those lookups per account so usernames and privacy
-- settings can't be probed without limit. The counter shares the invitation
-- limiter's row (already granted to the runtime role); invitations keep their
-- own window.
ALTER TABLE space_invite_limits
    ADD COLUMN direct_window_start timestamptz NOT NULL DEFAULT now(),
    ADD COLUMN direct_attempts integer NOT NULL DEFAULT 0;

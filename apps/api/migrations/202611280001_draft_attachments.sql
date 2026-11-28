-- Files staged in a synced draft, in order, and which of them are marked as
-- spoilers: external IDs of the account's own ready, unsent uploads in the
-- draft's channel. While a draft lists a file, the 24-hour cleanup of unsent
-- uploads keeps it, because a device that did not pick it cannot upload it
-- again. Lookups go through the (user_id, ...) unique index.
ALTER TABLE message_drafts
    ADD COLUMN attachment_ids text[] NOT NULL DEFAULT '{}',
    ADD COLUMN spoiler_ids text[] NOT NULL DEFAULT '{}';

-- Personal notes use the same private stream with one account in both slots.
-- Keep canonical ordering and uniqueness for ordinary two-person conversations.
ALTER TABLE direct_conversations DROP CONSTRAINT direct_conversations_check;
ALTER TABLE direct_conversations ADD CONSTRAINT direct_conversations_check
    CHECK (low_user_id <= high_user_id);

-- An account's quick reactions read its own reactions from the last 30 days;
-- without this they scan every reaction.
CREATE INDEX message_reactions_user_recent ON message_reactions (user_id, created_at);

-- The channel sequence a reaction was added at, so a reader whose read position
-- already covers it has seen it. Older reactions (and pods) leave it NULL.
ALTER TABLE message_reactions ADD COLUMN seq bigint;

-- Deletion scrubs retained events and reply quotes while holding the channel
-- lock. Find references directly instead of scanning all subsequent history.
-- Keep the original sequence bounds and atomic scrubbing unchanged.
CREATE INDEX channel_events_message_reference
    ON channel_events (channel_id, (payload->'message'->>'id'), seq)
    WHERE payload->'message'->>'id' IS NOT NULL;

CREATE INDEX messages_reply_reference
    ON messages (channel_id, (payload->'replyTo'->>'id'), channel_seq)
    WHERE payload->'replyTo'->>'id' IS NOT NULL;

CREATE INDEX channel_events_reply_reference
    ON channel_events (channel_id, (payload->'message'->'replyTo'->>'id'), seq)
    WHERE payload->'message'->'replyTo'->>'id' IS NOT NULL;

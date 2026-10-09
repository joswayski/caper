-- Account gateway recovery/checkpoints scan notification IDs, not timestamps.
CREATE INDEX notifications_user_cursor ON notifications (user_id, id);

-- Mark unread moves a conversation's read position back. Acknowledgements of
-- messages after the new position are revoked, not deleted, so the inbox and
-- unread counts show those messages again; acknowledging again clears it.
ALTER TABLE notification_inbox_reads ADD COLUMN revoked_at timestamptz;

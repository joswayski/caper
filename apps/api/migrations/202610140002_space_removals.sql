-- When the owner last removed this person from the space. It bars rejoining
-- through invite links until they accept a new invitation, which clears it.
-- `status='removed'` alone was forgotten once a later re-invitation was
-- cancelled or declined. Nullable and additive: older code ignores it.
ALTER TABLE space_invitations ADD COLUMN removed_at timestamptz;
UPDATE space_invitations SET removed_at = updated_at WHERE status = 'removed';

/** Marks messages from Caper admins. The server stamps `author.admin`, so a look-alike display name can't claim it. */
export default function AdminBadge() {
  return (
    <span className="chat-admin-badge" title="Caper admin">
      Admin
    </span>
  );
}

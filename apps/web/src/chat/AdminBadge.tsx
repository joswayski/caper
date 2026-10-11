/**
 * Marks messages from Caper admins, and ADMIN BOT for the Caper account's
 * automatic replies. The server stamps `author.admin`/`author.bot`, so a
 * look-alike display name can't claim them.
 */
export default function AdminBadge({ bot = false }: { bot?: boolean }) {
  return (
    <span className="chat-admin-badge" title={bot ? "Caper's automatic reply" : "Caper admin"}>
      {bot ? "Admin bot" : "Admin"}
    </span>
  );
}

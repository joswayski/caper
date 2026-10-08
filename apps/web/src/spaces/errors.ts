/** Sentences for the API's lowercase error text. Keys are the server's exact strings. */
const serverErrors: Record<string, string> = {
  "user not found": "User not found. Check the username and try again.",
  "account not found": "User not found. Check the username and try again.",
  "enter an exact username": "Enter an exact username.",
  "invalid username": "Use 3–32 lowercase letters, numbers, or underscores.",
  "user already in space": "This person is already a member.",
  "user already in channel": "This person already has access to this channel.",
  "user already invited": "This person already has a pending invitation.",
  "user must join the space first": "This person needs to join the space before you can add them to a channel.",
  "invitation cooldown; try again after 24 hours":
    "This person recently responded to an invitation. You can invite them again after 24 hours.",
  "too many invitation attempts; try again in 10 minutes": "Too many invitations. Try again in 10 minutes.",
  "pending invitation limit reached": "Too many invitations are waiting for a response. Try again later.",
  "membership limit reached": "You’ve reached the limit of spaces you can join. Leave one to join this space.",
  "space limit reached": "You’ve reached your space limit.",
  "channel limit reached": "This space has reached its channel limit.",
  "channel name already exists": "A channel with that name already exists.",
  "invalid channel name": "Use lowercase letters separated by single dashes.",
  "invalid space name": "Enter a space name up to 80 characters.",
  "owner cannot be removed": "The space owner can’t be removed.",
  "public channels are self-joined": "Anyone in the space can join a public channel without an invitation.",
  "resource not found": "That’s no longer available.",
  "channel not found": "This channel is no longer available.",
  "conversation not found": "This conversation is no longer available.",
  "request not found": "This message request is no longer available.",
  "you can't block yourself": "You can’t block yourself.",
  "too many blocked accounts": "You’ve blocked the maximum number of accounts.",
  "complete profile required": "Finish your profile first.",
  unauthorized: "You’re signed out. Sign in again to continue.",
  "spaces unavailable": "Caper is having trouble right now. Try again in a moment.",
  "messages unavailable": "Messages are unavailable right now. Try again in a moment.",
};

/** Capitalizes lowercase server text and ends it as a sentence: "a; b" reads "A. B." */
function sentence(message: string) {
  const text = message
    .trim()
    .split(/;\s*/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(". ");
  return /[.!?…]$/.test(text) ? text : `${text}.`;
}

/** Readable text for anything a spaces, channel or DM request can throw. */
export function friendlyError(error: unknown) {
  // AbortSignal.timeout() rejects with a DOMException, which isn't an Error in every runtime.
  const name = typeof error === "object" && error ? (error as { name?: unknown }).name : undefined;
  if (name === "TimeoutError" || name === "AbortError") return "That took too long. Try again.";
  // fetch() rejects with a TypeError when the network or server can't be reached.
  if (error instanceof TypeError) return "Couldn’t reach Caper. Check your connection.";
  if (!(error instanceof Error) || !error.message.trim()) return "That didn’t work. Try again.";
  return serverErrors[error.message] ?? sentence(error.message);
}

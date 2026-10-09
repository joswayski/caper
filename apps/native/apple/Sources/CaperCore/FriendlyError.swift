import Foundation

/// Web's `friendlyError` (apps/web/src/spaces/errors.ts): readable text for
/// anything a space, channel or DM request can throw. Display only: code that
/// branches on a failure reads `APIError.status` and `code`, never this text.
enum FriendlyError {
    static let fallback = "That didn’t work. Try again."
    static let unreachable = "Couldn’t reach Caper. Check your connection."
    static let timedOut = "That took too long. Try again."

    /// Sentences for the API's lowercase error text. Keys are the server's exact strings.
    static let serverErrors: [String: String] = [
        "user not found": "User not found. Check the username and try again.",
        "account not found": "User not found. Check the username and try again.",
        "enter an exact username": "Enter an exact username.",
        "invalid username": "Use 3–32 lowercase letters, numbers, or underscores.",
        "user already in space": "This person is already in the space.",
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
        "unauthorized": "You’re signed out. Sign in again to continue.",
        "spaces unavailable": "Caper is having trouble right now. Try again in a moment.",
        "messages unavailable": "Messages are unavailable right now. Try again in a moment.",
    ]

    static func message(for error: Error) -> String {
        if let failure = error as? URLError {
            switch failure.code {
            case .timedOut: return timedOut
            case .notConnectedToInternet, .cannotConnectToHost, .networkConnectionLost, .cannotFindHost, .dnsLookupFailed:
                return unreachable
            default: break
            }
        }
        // A stale request (the account or space changed meanwhile) has no text of its own.
        if error is CancellationError { return fallback }
        let text = (error as? APIError)?.message ?? error.localizedDescription
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return fallback }
        return serverErrors[text] ?? sentence(text)
    }

    /// Capitalizes lowercase server text and ends it as a sentence: "a; b" reads "A. B."
    static func sentence(_ message: String) -> String {
        let text = message.trimmingCharacters(in: .whitespacesAndNewlines)
            .components(separatedBy: ";")
            .map { $0.drop(while: \.isWhitespace) }
            .filter { !$0.isEmpty }
            .map { String($0.prefix(1)).uppercased() + String($0.dropFirst()) }
            .joined(separator: ". ")
        guard let last = text.last else { return fallback }
        return ".!?…".contains(last) ? text : text + "."
    }
}

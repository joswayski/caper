import Foundation
import Observation

/// A profile editor owns this snapshot. Previewing never mutates the account or
/// uploads bytes; only Save prepares an asset and publishes the complete profile.
@MainActor @Observable
public final class ProfileEditDraft {
    public let baseline: Account
    public var username: String
    public var displayName: String
    public private(set) var avatarId: Int?
    public private(set) var avatarAsset: String?
    public private(set) var avatarPicture: CaperPicture?
    public private(set) var preparedPicture: PreparedPicture?
    public private(set) var stagedAsset: String?
    public private(set) var saving = false

    public init(account: Account) {
        baseline = account
        username = account.username ?? ""
        displayName = account.displayName ?? ""
        avatarId = account.avatarId
        avatarAsset = account.avatarAsset
        avatarPicture = account.avatarPicture
    }

    public var avatarChanged: Bool {
        preparedPicture != nil || avatarId != baseline.avatarId || avatarAsset != baseline.avatarAsset
    }
    public var hasChanges: Bool {
        username != (baseline.username ?? "") || displayName != (baseline.displayName ?? "") || avatarChanged
    }
    public var canSave: Bool {
        !saving && hasChanges && ProfileValidation.error(username: username, displayName: displayName, saved: baseline.displayName) == nil
    }
    public enum CloseDecision: Equatable { case close, confirmDiscard, blocked }
    public var closeDecision: CloseDecision { saving ? .blocked : hasChanges ? .confirmDiscard : .close }

    public func usePicture(_ picture: PreparedPicture) {
        guard !saving else { return }
        preparedPicture = picture
        stagedAsset = nil
        avatarAsset = nil
        avatarPicture = nil
    }

    public func useHistory(_ picture: AvatarHistoryPicture) {
        guard !saving, let asset = picture.avatarAsset else { return }
        preparedPicture = nil
        stagedAsset = nil
        avatarAsset = asset
        avatarPicture = picture.avatarPicture
    }

    public func removePicture() {
        guard !saving else { return }
        preparedPicture = nil
        stagedAsset = nil
        avatarAsset = nil
        avatarPicture = nil
    }

    public func regenerate() {
        guard !saving else { return }
        let previous = CaperAvatar.index(for: avatarId)
        let candidate = Int.random(in: 0..<(previous == nil ? 800 : 799))
        avatarId = previous.map { candidate >= $0 ? candidate + 1 : candidate } ?? candidate
        removePicture()
    }

    /// A successful staged upload survives a failed publication so retrying
    /// doesn't reserve/upload the same photo again. No global identity is set here.
    public func save(api: APIClient, validateSession: () throws -> Void = {}) async throws -> Account {
        guard canSave else { throw CancellationError() }
        saving = true
        defer { saving = false }
        try validateSession()
        if let preparedPicture, stagedAsset == nil {
            do { stagedAsset = try await ProfilePictureUploader.stageAvatar(preparedPicture, api: api) }
            catch is CancellationError { throw CancellationError() }
            catch { throw UserFacingError(message: ProfilePicturePolicy.message(for: error, purpose: .avatar)) }
        }
        try validateSession()
        let avatar = avatarChanged ? ProfileAvatarInput(avatarId: CaperAvatar.index(for: avatarId) ?? 0,
            avatarAsset: preparedPicture == nil ? avatarAsset : stagedAsset,
            expectedAvatarId: baseline.avatarId, expectedAvatarAsset: baseline.avatarAsset) : nil
        return try await api.updateProfile(username: username, displayName: displayName, avatar: avatar)
    }

    public static func saveError(_ error: Error) -> String {
        if let failure = error as? UserFacingError { return failure.message }
        if let failure = error as? APIError {
            if failure.code == "profile_changed" { return "Your profile changed elsewhere. Your draft is still here; close and reopen to refresh before saving." }
            if failure.status == 409 { return "That username is already taken." }
            if failure.status == 400 { return "Check the username and display name requirements." }
        }
        return "Your profile could not be saved. Your changes are still here. Please try again."
    }
}

/// Null expected values are meaningful compare-and-set baselines, not omitted fields.
public struct ProfileAvatarInput: Encodable, Sendable {
    public let avatarId: Int
    public let avatarAsset: String?
    public let expectedAvatarId: Int?
    public let expectedAvatarAsset: String?

    enum CodingKeys: String, CodingKey { case avatarId, avatarAsset, expectedAvatarId, expectedAvatarAsset }
    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(avatarId, forKey: .avatarId)
        try values.encode(avatarAsset, forKey: .avatarAsset)
        try values.encode(expectedAvatarId, forKey: .expectedAvatarId)
        try values.encode(expectedAvatarAsset, forKey: .expectedAvatarAsset)
    }
}

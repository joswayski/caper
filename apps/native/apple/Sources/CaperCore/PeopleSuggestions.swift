import SwiftUI

/// Suggestions under the exact-username fields: space invites, private channel
/// grants and new direct messages. They rank people the client already has with
/// the `@` mention ranking. A row only fills in the username; the server still
/// looks it up exactly.
enum PeopleSuggestions {
    /// The field's text trimmed, without one leading `@`, lowercased; nil when
    /// nothing is left, which shows no list.
    static func query(_ text: String) -> String? {
        var query = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if query.hasPrefix("@") { query.removeFirst() }
        return query.isEmpty ? nil : query.lowercased()
    }

    /// Up to six candidates for the field's text, best first.
    static func match(text: String, candidates: [MentionCandidate]) -> [MentionCandidate] {
        guard let query = query(text) else { return [] }
        return Array(MentionAutocomplete.rankPeople(query: query, candidates).prefix(MentionAutocomplete.maximumChoices))
    }

    /// Space invites: the people you know (`GET /api/people`), except anyone
    /// already in the space or invited to it, matched by account id.
    static func spaceInvite(people: [Person]?, members: [Member], invited: [Member], accountID: String?) -> [MentionCandidate] {
        let excluded = Set(members.map(\.id) + invited.map(\.id) + [accountID].compactMap { $0 })
        return (people ?? []).filter { !excluded.contains($0.id) }.map { MentionCandidate($0) }
    }

    /// Private channel grants: the space's members, since only they can be
    /// granted, except you, anyone with access and anyone invited.
    static func channelGrant(spaceMembers: [Member], channelMembers: [Member], invited: [Member],
                             accountID: String?) -> [MentionCandidate] {
        let excluded = Set(channelMembers.map(\.id) + invited.map(\.id) + [accountID].compactMap { $0 })
        return spaceMembers.filter { !excluded.contains($0.id) }.map { MentionCandidate($0) }
    }

    /// New direct messages: the people you know, never yourself.
    static func directMessage(people: [Person]?, accountID: String?) -> [MentionCandidate] {
        (people ?? []).filter { $0.id != accountID }.map { MentionCandidate($0) }
    }
}

/// One exact-username field's list: the highlighted row, and the text it was
/// closed for (a picked username, or Escape). It stays closed until the text
/// changes.
struct PeopleSuggestionState: Equatable {
    var highlighted = 0
    var closedFor: String?

    /// The rows under the field: none unless it has focus and the list is open.
    func choices(text: String, candidates: [MentionCandidate], focused: Bool) -> [MentionCandidate] {
        guard focused, closedFor != text else { return [] }
        return PeopleSuggestions.match(text: text, candidates: candidates)
    }

    func highlightedIndex(count: Int) -> Int { min(max(highlighted, 0), max(count - 1, 0)) }

    /// The row Return picks.
    func highlightedChoice(in choices: [MentionCandidate]) -> MentionCandidate? {
        choices.isEmpty ? nil : choices[highlightedIndex(count: choices.count)]
    }

    /// Up and Down wrap around, as in the composer.
    mutating func move(down: Bool, count: Int) {
        guard count > 0 else { return }
        highlighted = (highlightedIndex(count: count) + (down ? 1 : count - 1)) % count
    }

    /// After a pick (with the field's new text) or Escape.
    mutating func close(for text: String) { closedFor = text; highlighted = 0 }

    /// An edit reopens the list on its first row.
    mutating func textChanged(to text: String) {
        if text != closedFor { closedFor = nil }
        highlighted = 0
    }
}

/// Keyboard control of the list from its field, as in the composer: Up and
/// Down move the highlight and Escape closes the list (on the Mac without
/// closing the dialog). Return reaches the field's `onSubmit`, which picks the
/// highlighted row while the list is open.
struct PeopleSuggestionKeys: ViewModifier {
    let text: String
    /// The rows showing; keys pass through while there are none.
    let count: Int
    @Binding var state: PeopleSuggestionState
    func body(content: Content) -> some View {
        content
            .onChange(of: text) { _, edited in state.textChanged(to: edited) }
            .onKeyPress(keys: [.upArrow, .downArrow, .escape]) { press in
                guard count > 0 else { return .ignored }
                if press.key == .escape { closeList() } else { state.move(down: press.key == .downArrow, count: count) }
                return .handled
            }
            #if os(macOS)
            // The dialog closes on Escape; the field keeps it while the list is open.
            .onExitCommand(perform: count > 0 ? closeList : nil)
            #endif
    }
    private func closeList() { state.close(for: text) }
}

/// The list under an exact-username field, inline in its form so the dialog
/// never clips it: the composer's person rows, at most six. As in the composer,
/// four rows show at once and the highlighted row scrolls into view, so the
/// list stays above the iPhone keyboard.
struct PeopleSuggestionList: View {
    let choices: [MentionCandidate]
    let state: PeopleSuggestionState
    /// The list's accessibility identifier.
    let identifier: String
    let pick: (MentionCandidate) -> Void
    var body: some View {
        let highlighted = state.highlightedIndex(count: choices.count)
        ScrollViewReader { proxy in
            ScrollView {
                VStack(spacing: 0) {
                    ForEach(Array(choices.enumerated()), id: \.element.id) { index, candidate in
                        Button { pick(candidate) } label: {
                            MentionSuggestionLabel(candidate: candidate)
                                .modifier(SuggestionRow(selected: index == highlighted))
                        }.buttonStyle(.plain)
                            .accessibilityLabel(ComposerSuggestionsView.mentionLabel(candidate))
                            .accessibilityIdentifier("person-suggestion-\(candidate.username.lowercased())")
                            .id(candidate.id)
                    }
                }
            }.frame(height: min(CGFloat(choices.count) * 44, 176))
                .onChange(of: highlighted) { _, index in
                    if choices.indices.contains(index) { proxy.scrollTo(choices[index].id, anchor: .center) }
                }
        }
        .frame(maxWidth: .infinity)
        .background(CaperTheme.surface).clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(identifier)
    }
}

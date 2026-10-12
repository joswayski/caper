package chat.caper.android

import chat.caper.android.model.Member
import chat.caper.android.model.Person
import java.util.Locale

// People suggestions for the exact-username fields: space invites, private-channel
// access and new DMs. Choosing one only fills in its username; the server lookup
// stays exact and nothing is sent until the field's own button.

/** A username field's query: trimmed, one leading `@` dropped, lowercase. */
internal fun usernameQuery(text: String): String = text.trim().removePrefix("@").lowercase(Locale.ROOT)

/** Up to [limit] of [people] ranked like `@mention` suggestions; none for an empty query. */
internal fun usernameSuggestions(people: List<MentionCandidate>, text: String, limit: Int = 6): List<MentionCandidate> {
    val query = usernameQuery(text)
    return if (query.isEmpty()) emptyList() else rankPeople(people, query).take(limit)
}

private fun Person.candidate() = MentionCandidate(username, displayName, avatarId = avatarId)
private fun Member.candidate() = MentionCandidate(username, displayName, avatarId = avatarId)

/** Space invites: `/api/people` without the space's members or pending invitations (by id). */
internal fun spaceInviteCandidates(people: List<Person>?, members: List<Member>, invitations: List<Member>): List<MentionCandidate> {
    val excluded = (members + invitations).mapTo(HashSet()) { it.id }
    return people.orEmpty().filter { it.id !in excluded }.map { it.candidate() }
}

/**
 * Private-channel access: the space's [members] (the server only grants those) without
 * you, people who already have access or a pending invitation (by id).
 */
internal fun channelGrantCandidates(members: List<Member>, self: String?, grants: List<Member>, invitations: List<Member>): List<MentionCandidate> {
    val excluded = (grants + invitations).mapTo(HashSet()) { it.id }
    return members.filter { it.id != self && it.id !in excluded }.map { it.candidate() }
}

/** New DMs: everyone from `/api/people`, never you. */
internal fun directCandidates(people: List<Person>?, self: String?): List<MentionCandidate> =
    people.orEmpty().filter { it.id != self }.map { it.candidate() }

package chat.caper.android

import java.io.File
import java.util.concurrent.Executor
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * Unsent composer text of each conversation by channel or DM ID, and thread replies by [thread] key, so switching
 * conversations and coming back restores it. Like web's `localStorage`, it is also kept on this device for the
 * signed-in account (one JSON file each, outside backups), so quitting, a restart or an update restores it too.
 * Signing out deletes the account's file. Text only: staged files and reply targets stay in memory. Drafts are
 * never logged or sent to the server.
 */
internal class ConversationDrafts(private val writer: Executor = Executor { it.run() }) {
    private val drafts = mutableMapOf<String, String>()
    private var account = ""
    private var file: File? = null

    /** Opens [accountId]'s saved drafts in [dir] unless they are the ones open. A missing or unreadable file means none. */
    fun open(dir: File?, accountId: String) {
        if (accountId == account) return
        account = accountId
        drafts.clear()
        file = dir?.takeIf { accountId.isNotEmpty() && accountId.all { it.isLetterOrDigit() || it == '-' || it == '_' } }
            ?.let { File(it, "$accountId.json") }
        val saved = runCatching { file?.takeIf { it.exists() }?.readText()?.let { Json.parseToJsonElement(it).jsonObject } }.getOrNull()
        saved?.forEach { (key, value) -> runCatching { value.jsonPrimitive.contentOrNull }.getOrNull()?.takeIf { it.isNotBlank() }?.let { drafts[key] = it } }
    }

    operator fun get(conversation: String): String = drafts[conversation].orEmpty()

    operator fun set(conversation: String, text: String) {
        val changed = if (text.isBlank()) drafts.remove(conversation) != null else drafts.put(conversation, text) != text
        if (changed) save()
    }

    /** Signing out: every draft goes, and the account's file with them, so another account never sees them. */
    fun clear() {
        drafts.clear()
        account = ""
        val gone = file
        file = null
        if (gone != null) writer.execute { gone.delete() }
    }

    private fun save() {
        val target = file ?: return
        val snapshot = JsonObject(drafts.mapValues { JsonPrimitive(it.value) }).toString()
        val empty = drafts.isEmpty()
        writer.execute {
            // Write beside it, then rename: a crash never leaves half a file. Never log the drafts or their path.
            runCatching {
                if (empty) target.delete()
                else {
                    target.parentFile?.mkdirs()
                    val temporary = File(target.path + ".tmp")
                    temporary.writeText(snapshot)
                    if (!temporary.renameTo(target)) { target.delete(); temporary.renameTo(target) }
                }
            }
        }
    }

    companion object {
        /** A thread reply's key: root message IDs never collide with channel IDs. */
        fun thread(rootId: String) = "thread:$rootId"
    }
}

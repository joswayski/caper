package chat.caper.android.push

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import androidx.compose.ui.graphics.toArgb
import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.core.graphics.drawable.IconCompat
import androidx.core.graphics.drawable.toBitmap
import chat.caper.android.MainActivity
import chat.caper.android.R
import chat.caper.android.caperAvatarResources
import chat.caper.android.data.TokenStore
import chat.caper.android.ui.Terracotta
import java.io.File
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow

/** The DM or channel on screen while the app is in the foreground; its pushes show no notification. */
internal object ForegroundConversation {
    @Volatile var id: String? = null
}

/** Message pushes received while the app runs, shown or not: the open app refreshes unread state from them. */
internal object PushActivity {
    val received = MutableSharedFlow<Unit>(extraBufferCapacity = 1, onBufferOverflow = BufferOverflow.DROP_OLDEST)
}

/** Message notifications: one per DM or channel, tagged with its ID, in MessagingStyle. */
internal object CaperNotifications {
    const val EXTRA_CONVERSATION_ID = "conversationId"
    const val EXTRA_SPACE_ID = "spaceId"
    const val EXTRA_CHANNEL_ID = "channelId"
    const val EXTRA_KIND = "kind"
    private const val NOTIFICATION_ID = 1
    private const val MESSAGE_IDS = "chat.caper.android.messageIds"
    private const val MEDIA_FILES = "chat.caper.android.notificationMedia"
    private const val MAX_MESSAGES = 25
    // cancelAll (sign-out) invalidates every in-flight notification; cancel(tag) only its own conversation's.
    private var generation = 0L
    private val tagGenerations = HashMap<String, Long>()

    fun createChannels(context: Context) {
        context.getSystemService(NotificationManager::class.java).createNotificationChannels(pushChannels.map { (id, name) ->
            NotificationChannel(id, name, if (id == CHANNEL_MESSAGES_CHANNEL) NotificationManager.IMPORTANCE_DEFAULT else NotificationManager.IMPORTANCE_HIGH)
        })
    }

    fun show(context: Context, payload: PushPayload) {
        val epoch = synchronized(this) { generation to tagGenerations[payload.tag] }
        val session = TokenStore(context).read()
        val prepared = NotificationAttachment.load(context, payload.attachment)
        synchronized(this) {
            if (epoch != (generation to tagGenerations[payload.tag]) || session != TokenStore(context).read() || !PushRegistration.enabled(context) ||
                ForegroundConversation.id == payload.tag) {
                prepared?.delete()
                return
            }
            val media = prepared?.let { file ->
                val directory = File(context.cacheDir, "notification-media").apply { mkdirs() }
                val published = File(directory, file.name)
                if (file.renameTo(published)) published else { file.delete(); null }
            }
            if (!present(context, payload, media)) media?.delete()
        }
    }

    private fun present(context: Context, payload: PushPayload, media: File?): Boolean {
        if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return false
        val manager = context.getSystemService(NotificationManager::class.java)
        createChannels(context)
        val current = manager.activeNotifications.firstOrNull { it.tag == payload.tag && it.id == NOTIFICATION_ID }?.notification
        val seen = current?.extras?.getStringArrayList(MESSAGE_IDS).orEmpty()
        // FCM can deliver a message twice.
        if (payload.messageId in seen) return false
        // New messages join the conversation's notification.
        val style = current?.let { NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(it) }
            ?: NotificationCompat.MessagingStyle(Person.Builder().setName("You").build())
        val avatar = payload.senderAvatarId?.let { index ->
            ContextCompat.getDrawable(context, caperAvatarResources[index])?.toBitmap(128, 128)
        }
        val sender = Person.Builder().setName(payload.sender).setKey(payload.senderId)
            .setIcon(avatar?.let(IconCompat::createWithBitmap)).build()
        val message = NotificationCompat.MessagingStyle.Message(payload.body, System.currentTimeMillis(), sender)
        if (media != null) message.setData("image/png", FileProvider.getUriForFile(context, "${context.packageName}.notification-media", media))
        style.setConversationTitle(payload.conversationTitle).setGroupConversation(payload.groupConversation).addMessage(message)
        // The data URI keeps each conversation's PendingIntent (and its extras) separate.
        val open = Intent(context, MainActivity::class.java).setData(Uri.fromParts("caper", payload.tag, null))
            .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        if (payload.conversationId != null) open.putExtra(EXTRA_CONVERSATION_ID, payload.conversationId)
        else open.putExtra(EXTRA_SPACE_ID, payload.spaceId).putExtra(EXTRA_CHANNEL_ID, payload.channelId)
        open.putExtra(EXTRA_KIND, payload.kind)
        val pending = PendingIntent.getActivity(context, 0, open, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val dismiss = PendingIntent.getBroadcast(context, 0,
            Intent(context, NotificationDismissReceiver::class.java).setData(Uri.fromParts("caper-dismiss", payload.tag, null)),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val files = (current?.extras?.getStringArrayList(MEDIA_FILES).orEmpty() + listOfNotNull(media?.name)).takeLast(MAX_MESSAGES)
        val notification = NotificationCompat.Builder(context, payload.channel)
            .setSmallIcon(R.drawable.ic_caper_notification).setColor(Terracotta.toArgb())
            .setLargeIcon(avatar)
            .setStyle(if (payload.invitation) NotificationCompat.BigTextStyle().bigText(payload.body) else style).setContentTitle(payload.title).setContentText(payload.body)
            .setCategory(if (payload.invitation) NotificationCompat.CATEGORY_EVENT else NotificationCompat.CATEGORY_MESSAGE).setContentIntent(pending).setAutoCancel(true)
            .setDeleteIntent(dismiss).setTimeoutAfter(86_400_000)
            .addExtras(Bundle().apply {
                putStringArrayList(MESSAGE_IDS, ArrayList((seen + payload.messageId).takeLast(MAX_MESSAGES)))
                putStringArrayList(MEDIA_FILES, ArrayList(files))
            })
            .build()
        manager.notify(payload.tag, NOTIFICATION_ID, notification)
        // notify() enqueues asynchronously; activeNotifications can still be the old list.
        cleanup(context, files)
        return true
    }

    /** Clears a conversation's notification once it is on screen. */
    @Synchronized
    fun cancel(context: Context, tag: String) {
        tagGenerations[tag] = (tagGenerations[tag] ?: 0L) + 1
        context.getSystemService(NotificationManager::class.java).cancel(tag, NOTIFICATION_ID)
        cleanup(context)
    }

    /** Clears every message notification, never the voice call's. */
    @Synchronized
    fun cancelAll(context: Context) {
        generation++
        tagGenerations.clear()
        val manager = context.getSystemService(NotificationManager::class.java)
        val channels = pushChannels.map { it.first }.toSet()
        manager.activeNotifications.filter { it.notification.channelId in channels }.forEach { manager.cancel(it.tag, it.id) }
        cleanup(context)
    }

    @Synchronized
    fun cleanup(context: Context, pending: List<String> = emptyList()) {
        val retained = context.getSystemService(NotificationManager::class.java).activeNotifications
            .flatMap { it.notification.extras.getStringArrayList(MEDIA_FILES).orEmpty() }.toSet() + pending
        File(context.cacheDir, "notification-media").listFiles()?.filter {
            it.name !in retained || System.currentTimeMillis() - it.lastModified() > 86_400_000
        }?.forEach { it.delete() }
    }
}

/** System dismissal/timeout releases the app-private thumbnail after the OS used it. */
class NotificationDismissReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) { CaperNotifications.cleanup(context) }
}

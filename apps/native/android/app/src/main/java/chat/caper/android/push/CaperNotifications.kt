package chat.caper.android.push

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
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
import chat.caper.android.MainActivity
import chat.caper.android.R
import chat.caper.android.ui.Terracotta

/** The DM or channel on screen while the app is in the foreground; its pushes show no notification. */
internal object ForegroundConversation {
    @Volatile var id: String? = null
}

/** Message notifications: one per DM or channel, tagged with its ID, in MessagingStyle. */
internal object CaperNotifications {
    const val EXTRA_CONVERSATION_ID = "conversationId"
    const val EXTRA_SPACE_ID = "spaceId"
    const val EXTRA_CHANNEL_ID = "channelId"
    private const val NOTIFICATION_ID = 1
    private const val MESSAGE_IDS = "chat.caper.android.messageIds"
    private const val MAX_MESSAGES = 25

    fun createChannels(context: Context) {
        context.getSystemService(NotificationManager::class.java).createNotificationChannels(pushChannels.map { (id, name) ->
            NotificationChannel(id, name, if (id == CHANNEL_MESSAGES_CHANNEL) NotificationManager.IMPORTANCE_DEFAULT else NotificationManager.IMPORTANCE_HIGH)
        })
    }

    fun show(context: Context, payload: PushPayload) {
        if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        val manager = context.getSystemService(NotificationManager::class.java)
        createChannels(context)
        val current = manager.activeNotifications.firstOrNull { it.tag == payload.tag && it.id == NOTIFICATION_ID }?.notification
        val seen = current?.extras?.getStringArrayList(MESSAGE_IDS).orEmpty()
        // FCM can deliver a message twice.
        if (payload.messageId in seen) return
        // New messages join the conversation's notification.
        val style = current?.let { NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(it) }
            ?: NotificationCompat.MessagingStyle(Person.Builder().setName("You").build())
        style.setConversationTitle(payload.conversationTitle).setGroupConversation(payload.groupConversation)
            .addMessage(payload.body, System.currentTimeMillis(), Person.Builder().setName(payload.sender).setKey(payload.senderId).build())
        // The data URI keeps each conversation's PendingIntent (and its extras) separate.
        val open = Intent(context, MainActivity::class.java).setData(Uri.fromParts("caper", payload.tag, null))
            .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        if (payload.conversationId != null) open.putExtra(EXTRA_CONVERSATION_ID, payload.conversationId)
        else open.putExtra(EXTRA_SPACE_ID, payload.spaceId).putExtra(EXTRA_CHANNEL_ID, payload.channelId)
        val pending = PendingIntent.getActivity(context, 0, open, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val notification = NotificationCompat.Builder(context, payload.channel)
            .setSmallIcon(R.drawable.ic_caper_notification).setColor(Terracotta.toArgb())
            .setStyle(style).setContentTitle(payload.title).setContentText(payload.body)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE).setContentIntent(pending).setAutoCancel(true)
            .addExtras(Bundle().apply { putStringArrayList(MESSAGE_IDS, ArrayList((seen + payload.messageId).takeLast(MAX_MESSAGES))) })
            .build()
        manager.notify(payload.tag, NOTIFICATION_ID, notification)
    }

    /** Clears a conversation's notification once it is on screen. */
    fun cancel(context: Context, tag: String) {
        context.getSystemService(NotificationManager::class.java).cancel(tag, NOTIFICATION_ID)
    }

    /** Clears every message notification, never the voice call's. */
    fun cancelAll(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        val channels = pushChannels.map { it.first }.toSet()
        manager.activeNotifications.filter { it.notification.channelId in channels }.forEach { manager.cancel(it.tag, it.id) }
    }
}

package chat.caper.android.push

import android.Manifest
import android.app.NotificationManager
import android.os.Build
import android.os.Bundle
import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import chat.caper.android.R
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** Exercises the real system notification snapshot, not a launcher/vendor badge API. */
@RunWith(AndroidJUnit4::class)
class NotificationBadgeTest {
    @Test fun retractionsCountMessagesOutsideTheVisibleHistoryAndReadPollingClearsTheBadge() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        if (Build.VERSION.SDK_INT >= 33) instrumentation.uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.POST_NOTIFICATIONS)
        val manager = context.getSystemService(NotificationManager::class.java)
        CaperNotifications.createChannels(context)
        assertTrue(manager.getNotificationChannel(DIRECT_MESSAGES_CHANNEL).canShowBadge())
        val tag = "badge-test"
        val ids = (1..40).map { "message-$it" }
        val style = NotificationCompat.MessagingStyle(Person.Builder().setName("You").build())
        ids.forEach { id ->
            val line = NotificationCompat.MessagingStyle.Message(id, System.currentTimeMillis(), Person.Builder().setName("Sender").build())
            line.extras.putString("chat.caper.android.messageId", id)
            style.addMessage(line)
        }
        val postedAt = System.currentTimeMillis() - 1_000
        try {
            manager.notify(tag, 1, NotificationCompat.Builder(context, DIRECT_MESSAGES_CHANNEL)
                .setSmallIcon(R.drawable.ic_caper_notification).setStyle(style).setNumber(40).setWhen(postedAt)
                .addExtras(Bundle().apply { putStringArrayList("chat.caper.android.messageIds", ArrayList(ids)) }).build())
            awaitCount(manager, tag, 40)
            // Message 1 has fallen out of MessagingStyle's 25 lines, but is still counted.
            CaperNotifications.retract(context, PushRetraction(ids.first(), tag, null, null))
            awaitCount(manager, tag, 39)
            CaperNotifications.retract(context, PushRetraction(ids.last(), tag, null, null))
            awaitCount(manager, tag, 38)
            CaperNotifications.retract(context, PushRetraction(ids.first(), tag, null, null))
            awaitCount(manager, tag, 38)
            CaperNotifications.reconcileReadDirect(context, tag, postedAt - 1)
            awaitCount(manager, tag, 38) // A stale read response cannot clear a newer push.
            CaperNotifications.reconcileReadDirect(context, tag, System.currentTimeMillis())
            awaitCount(manager, tag, null)
        } finally {
            CaperNotifications.cancel(context, tag)
        }
    }

    private fun awaitCount(manager: NotificationManager, tag: String, expected: Int?) {
        repeat(100) {
            val actual = manager.activeNotifications.firstOrNull { it.tag == tag }?.notification?.number
            if (actual == expected) return
            Thread.sleep(20)
        }
        assertEquals(expected, manager.activeNotifications.firstOrNull { it.tag == tag }?.notification?.number)
    }
}

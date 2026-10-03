package chat.caper.android.push

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.ContextCompat
import androidx.core.app.NotificationCompat
import chat.caper.android.BuildConfig
import chat.caper.android.MainActivity
import chat.caper.android.R
import chat.caper.android.data.CaperApi
import chat.caper.android.data.TokenStore
import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.tasks.await
import kotlinx.coroutines.withTimeoutOrNull
import java.security.MessageDigest

object PushRegistration {
    private const val PREFS = "push_preferences"
    private const val ENABLED = "enabledSession"
    private fun sessionKey(token: String) = MessageDigest.getInstance("SHA-256").digest(token.toByteArray()).joinToString("") { "%02x".format(it) }

    fun enabled(context: Context): Boolean {
        if (!BuildConfig.FIREBASE_ENABLED) return false
        val token = TokenStore(context).read() ?: return false
        return context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(ENABLED, null) == sessionKey(token)
    }

    suspend fun enable(context: Context) {
        if (!BuildConfig.FIREBASE_ENABLED) return
        val accountToken = TokenStore(context).read() ?: return
        check(context.getSystemService(NotificationManager::class.java).areNotificationsEnabled()) { "Allow notifications in Android settings first." }
        if ("fcm" !in CaperApi().pushConfig(accountToken).platforms) return
        val messaging = FirebaseMessaging.getInstance()
        messaging.isAutoInitEnabled = true
        val deviceToken = messaging.token.await()
        if (TokenStore(context).read() != accountToken) return
        CaperApi().registerPush(accountToken, deviceToken)
        if (TokenStore(context).read() == accountToken) context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(ENABLED, sessionKey(accountToken)).apply()
    }

    suspend fun register(context: Context, deviceToken: String) {
        if (!enabled(context)) return
        TokenStore(context).read()?.let { CaperApi().registerPush(it, deviceToken) }
    }

    suspend fun disable(context: Context, accountToken: String?) {
        if (!BuildConfig.FIREBASE_ENABLED || accountToken == null) return
        val preferences = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        if (preferences.getString(ENABLED, null) != sessionKey(accountToken)) return
        preferences.edit().remove(ENABLED).apply()
        FirebaseMessaging.getInstance().isAutoInitEnabled = false
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.activeNotifications.filter { it.tag == "direct_messages" }.forEach { manager.cancel(it.tag, it.id) }
        withTimeoutOrNull(10_000) {
            val deviceToken = runCatching { FirebaseMessaging.getInstance().token.await() }.getOrNull()
            if (deviceToken != null) runCatching { CaperApi().unregisterPush(accountToken, deviceToken) }
        }
    }
}

class CaperMessagingService : FirebaseMessagingService() {
    override fun onNewToken(token: String) {
        CoroutineScope(Dispatchers.IO).launch { runCatching { PushRegistration.register(applicationContext, token) } }
    }

    override fun onMessageReceived(message: RemoteMessage) {
        if (!PushRegistration.enabled(this)) return
        val conversation = message.data["conversationId"] ?: return
        val messageId = message.data["messageId"] ?: return
        if (!Regex("^[A-Za-z0-9]{12}$").matches(conversation) || !Regex("^[A-Za-z0-9]{15}$").matches(messageId)) return
        val accountToken = TokenStore(this).read() ?: return
        CoroutineScope(Dispatchers.IO).launch {
            // Provider queues can outlive logout. Check the current account's
            // access before displaying an alert for a previous account's DM.
            val accessible = runCatching { CaperApi().directConversations(accountToken).conversations.any { it.id == conversation } }.getOrDefault(false)
            if (!accessible || !PushRegistration.enabled(applicationContext) || TokenStore(applicationContext).read() != accountToken) return@launch
            if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(this@CaperMessagingService, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return@launch
            val manager = getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(NotificationChannel("direct_messages", "Direct messages", NotificationManager.IMPORTANCE_DEFAULT))
            val intent = Intent(this@CaperMessagingService, MainActivity::class.java).putExtra("conversationId", conversation)
                .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            val pending = PendingIntent.getActivity(this@CaperMessagingService, conversation.hashCode(), intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            manager.notify("direct_messages", messageId.hashCode(), NotificationCompat.Builder(this@CaperMessagingService, "direct_messages")
                .setSmallIcon(R.drawable.ic_caper_notification).setContentTitle("Caper")
                .setContentText("You have a new direct message.").setContentIntent(pending).setAutoCancel(true).build())
        }
    }
}

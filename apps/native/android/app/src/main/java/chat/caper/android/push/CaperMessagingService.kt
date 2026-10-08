package chat.caper.android.push

import android.app.NotificationManager
import android.content.Context
import chat.caper.android.BuildConfig
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

/** What opening the app does about push for an account that hasn't turned it on on this phone. */
enum class PushOffer { NONE, ENABLE, ASK }

/**
 * Push is on by default. Opening the app turns it on when Android already allows
 * notifications, or asks once (Android 13+), unless the account turned it off here
 * or the server doesn't offer FCM.
 */
fun pushOffer(firebase: Boolean, turnedOff: Boolean, offered: Boolean, permitted: Boolean, asked: Boolean): PushOffer = when {
    !firebase || turnedOff || !offered -> PushOffer.NONE
    permitted -> PushOffer.ENABLE
    asked -> PushOffer.NONE
    else -> PushOffer.ASK
}

object PushRegistration {
    private const val PREFS = "push_preferences"
    private const val ENABLED = "enabledSession"
    /** Per account: turned push off in User settings on this phone. Logout doesn't set it. */
    private const val TURNED_OFF = "turnedOff:"
    /** Per account: already shown the Android 13+ prompt, so app open never asks again. */
    private const val ASKED = "asked:"
    private fun sessionKey(token: String) = MessageDigest.getInstance("SHA-256").digest(token.toByteArray()).joinToString("") { "%02x".format(it) }
    private fun preferences(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun turnedOff(context: Context, accountId: String) = preferences(context).getBoolean(TURNED_OFF + accountId, false)
    fun setTurnedOff(context: Context, accountId: String, off: Boolean) {
        preferences(context).edit().putBoolean(TURNED_OFF + accountId, off).apply()
    }
    fun asked(context: Context, accountId: String) = preferences(context).getBoolean(ASKED + accountId, false)
    fun markAsked(context: Context, accountId: String) { preferences(context).edit().putBoolean(ASKED + accountId, true).apply() }

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
        CaperNotifications.createChannels(context)
        CaperApi().registerPush(accountToken, deviceToken, BuildConfig.APPLICATION_ID)
        if (TokenStore(context).read() == accountToken) context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(ENABLED, sessionKey(accountToken)).apply()
    }

    suspend fun register(context: Context, deviceToken: String) {
        if (!enabled(context)) return
        TokenStore(context).read()?.let { CaperApi().registerPush(it, deviceToken, BuildConfig.APPLICATION_ID) }
    }

    suspend fun disable(context: Context, accountToken: String?) {
        if (!BuildConfig.FIREBASE_ENABLED || accountToken == null) return
        val preferences = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        if (preferences.getString(ENABLED, null) != sessionKey(accountToken)) return
        preferences.edit().remove(ENABLED).apply()
        FirebaseMessaging.getInstance().isAutoInitEnabled = false
        CaperNotifications.cancelAll(context)
        withTimeoutOrNull(10_000) {
            val deviceToken = runCatching { FirebaseMessaging.getInstance().token.await() }.getOrNull()
            if (deviceToken != null) runCatching { CaperApi().unregisterPush(accountToken, deviceToken, BuildConfig.APPLICATION_ID) }
        }
    }
}

class CaperMessagingService : FirebaseMessagingService() {
    override fun onNewToken(token: String) {
        CoroutineScope(Dispatchers.IO).launch { runCatching { PushRegistration.register(applicationContext, token) } }
    }

    override fun onMessageReceived(message: RemoteMessage) {
        // Provider queues can outlive logout or an account switch: only the sign-in
        // session that turned notifications on shows them. The server already skips
        // requests, blocks, mutes and levels, so no per-push API call is needed.
        if (!PushRegistration.enabled(this)) return
        val payload = PushPayload.parse(message.data) ?: return
        if (ForegroundConversation.id == payload.tag) return
        CaperNotifications.show(this, payload)
    }
}

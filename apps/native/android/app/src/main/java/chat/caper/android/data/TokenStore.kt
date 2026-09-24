package chat.caper.android.data

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import chat.caper.android.BuildConfig
import java.nio.charset.StandardCharsets
import java.security.MessageDigest
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Stores only ciphertext in SharedPreferences; the AES key never leaves Android Keystore. */
class TokenStore(context: Context, apiUrl: String = BuildConfig.API_BASE_URL) {
    private val preferences = context.getSharedPreferences("secure_session", Context.MODE_PRIVATE)
    private val namespace = MessageDigest.getInstance("SHA-256")
        .digest(canonicalApiOrigin(apiUrl).toByteArray(StandardCharsets.UTF_8))
        .joinToString("") { "%02x".format(it) }
    private val preferenceKey = "bearer_$namespace"
    private val alias = "caper-account-token-v1-$namespace"

    @Synchronized fun read(): String? {
        val encoded = preferences.getString(preferenceKey, null) ?: return null
        return runCatching {
            val bytes = Base64.decode(encoded, Base64.NO_WRAP)
            require(bytes.size > 12)
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes, 0, 12))
            String(cipher.doFinal(bytes, 12, bytes.size - 12), StandardCharsets.UTF_8)
        }.getOrElse {
            clear()
            null
        }
    }

    @Synchronized fun write(token: String) {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key())
        val encrypted = cipher.doFinal(token.toByteArray(StandardCharsets.UTF_8))
        val value = Base64.encodeToString(cipher.iv + encrypted, Base64.NO_WRAP)
        check(preferences.edit().putString(preferenceKey, value).commit())
    }

    @Synchronized fun clear() {
        preferences.edit().remove(preferenceKey).commit()
    }

    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(alias, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
            init(
                KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .setKeySize(256)
                    .build(),
            )
            generateKey()
        }
    }
}

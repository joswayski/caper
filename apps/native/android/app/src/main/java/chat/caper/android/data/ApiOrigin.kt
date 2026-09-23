package chat.caper.android.data

import okhttp3.HttpUrl.Companion.toHttpUrlOrNull

internal fun canonicalApiOrigin(value: String): String {
    val url = value.trim().toHttpUrlOrNull() ?: throw IllegalArgumentException("Invalid API URL.")
    require(url.username.isEmpty() && url.password.isEmpty()) { "API URL must not contain credentials." }
    val loopback = url.host == "localhost" || url.host.endsWith(".localhost") || url.host == "127.0.0.1" || url.host == "::1"
    require(url.isHttps || loopback) { "API URL must use HTTPS outside loopback development." }
    return url.newBuilder().encodedPath("/").query(null).fragment(null).build().toString().removeSuffix("/")
}

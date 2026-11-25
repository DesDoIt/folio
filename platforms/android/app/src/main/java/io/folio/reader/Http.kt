package io.folio.reader

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlin.coroutines.coroutineContext
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.nio.charset.Charset

/** HTTP Basic login for a catalog. */
data class Login(val user: String, val password: String)

/** A fetched document. */
class Fetched(val url: String, val bytes: ByteArray, val contentType: String?) {
    /** The body as text: charset from the header, else from an XML/HTML declaration, else UTF-8. */
    val text: String by lazy {
        val fromHeader = contentType?.substringAfter("charset=", "")?.substringBefore(';')?.trim('"', ' ')
        val head = String(bytes, 0, minOf(bytes.size, 1024), Charsets.ISO_8859_1)
        val fromDoc = Regex("""(?:encoding|charset)\s*=\s*["']?([A-Za-z0-9_\-]+)""").find(head)?.groupValues?.get(1)
        val cs = listOfNotNull(fromHeader, fromDoc).firstNotNullOfOrNull {
            try { Charset.forName(it) } catch (e: Exception) { null }
        } ?: Charsets.UTF_8
        String(bytes, cs).removePrefix("﻿")
    }
}

/** HTTP over the phone's own network stack (its proxy, VPN, certificates and data settings). */
object Http {
    private const val USER_AGENT = "Folio/0.1 (Android; reader)"
    private const val MAX_BYTES = 12 shl 20
    private const val MAX_DOWNLOAD = 400L shl 20

    suspend fun get(url: String, maxBytes: Int = MAX_BYTES, login: Login? = null): Fetched = withContext(Dispatchers.IO) {
        val out = ByteArrayOutputStream()
        val (final, type) = open(url, login) { _, input ->
            val buf = ByteArray(1 shl 15)
            while (true) {
                val n = input.read(buf)
                if (n < 0) break
                out.write(buf, 0, n)
                if (out.size() > maxBytes) throw IOException("The response is too large.")
            }
        }
        Fetched(final, out.toByteArray(), type)
    }

    /**
     * Download to a file, reporting (bytes so far, total or -1). The file is written next to
     * [dest] and moved into place only when complete. Returns the final address and content type.
     */
    suspend fun download(url: String, dest: java.io.File, login: Login? = null, onProgress: (Long, Long) -> Unit = { _, _ -> }):
        Pair<String, String?> = withContext(Dispatchers.IO) {
        val part = java.io.File(dest.parentFile, dest.name + ".part")
        val job = coroutineContext[kotlinx.coroutines.Job]
        try {
            val r = open(url, login) { total, input ->
                part.outputStream().use { out ->
                    val buf = ByteArray(1 shl 16)
                    var done = 0L
                    var last = 0L
                    while (true) {
                        job?.ensureActive() // stop promptly when the download is cancelled
                        val n = input.read(buf)
                        if (n < 0) break
                        out.write(buf, 0, n)
                        done += n
                        if (done > MAX_DOWNLOAD) throw IOException("The file is too large.")
                        if (done - last > 64 * 1024) { onProgress(done, total); last = done }
                    }
                    onProgress(done, total)
                }
            }
            if (!part.renameTo(dest)) throw IOException("Couldn't save the file.")
            r
        } finally {
            part.delete()
        }
    }

    /** Open a connection (following redirects, also across http -> https) and read its body. */
    private fun <T> open(url: String, login: Login?, read: (Long, java.io.InputStream) -> T): Pair<String, String?> {
        var current = url
        repeat(6) {
            val c = URL(current).openConnection() as HttpURLConnection
            try {
                c.instanceFollowRedirects = false
                c.connectTimeout = 6_000 // per address: a dead IPv6 route falls back to IPv4 quickly
                c.readTimeout = 30_000
                c.setRequestProperty("User-Agent", USER_AGENT)
                c.setRequestProperty("Accept", "application/rss+xml, application/atom+xml, application/xml;q=0.9, text/html;q=0.8, */*;q=0.5")
                // send the login only to the catalog's own host, never to where it redirects
                if (login != null && URL(current).host == URL(url).host) {
                    val token = android.util.Base64.encodeToString("${login.user}:${login.password}".toByteArray(), android.util.Base64.NO_WRAP)
                    c.setRequestProperty("Authorization", "Basic $token")
                }
                val code = c.responseCode
                if (code in 300..399) {
                    val loc = c.getHeaderField("Location") ?: throw IOException("Redirect without a target")
                    current = URL(URL(current), loc).toString()
                    return@repeat
                }
                if (code == 401) throw IOException(if (login == null) "This catalog needs a user name and password." else "The user name or password was not accepted.")
                if (code !in 200..299) throw IOException(httpMessage(code))
                c.inputStream.use { read(c.contentLengthLong, it) }
                return current to c.contentType
            } finally {
                c.disconnect()
            }
        }
        throw IOException("Too many redirects.")
    }

    private fun httpMessage(code: Int) = when (code) {
        401, 403 -> "The site refused access (HTTP $code)."
        404, 410 -> "Nothing at this address (HTTP $code)."
        429 -> "The site asked to slow down; try again later."
        503 -> "The site is down for maintenance or busy; try again later."
        in 500..599 -> "The site is having trouble (HTTP $code)."
        else -> "HTTP error $code."
    }
}

/** A short, human message for a network failure. */
fun Throwable.friendly(): String = when (this) {
    is java.net.UnknownHostException -> "No connection, or the site's address couldn't be found."
    is java.net.SocketTimeoutException -> "The site took too long to answer."
    is javax.net.ssl.SSLException -> "A secure connection to the site couldn't be made."
    // never show coroutine internals ("Timed out waiting for…", "…was cancelled")
    is kotlinx.coroutines.TimeoutCancellationException -> "The site took too long to answer."
    is kotlinx.coroutines.CancellationException -> "Stopped."
    else -> message ?: "Something went wrong."
}

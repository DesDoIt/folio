package io.folio.reader

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import uniffi.engine_ffi.FeedItem
import uniffi.engine_ffi.feedAddress
import uniffi.engine_ffi.findFeedLink
import uniffi.engine_ffi.looksLikeFeed
import uniffi.engine_ffi.parseFeed
import java.io.File
import java.security.MessageDigest
import java.util.Calendar
import java.util.concurrent.TimeUnit

data class Subscription(
    val url: String,
    val title: String,
    /** notify about new articles from this feed (when notifications are on) */
    val notify: Boolean = true,
    val added: Long = System.currentTimeMillis(),
    val checked: Long = 0,
    val error: String? = null,
) {
    fun toJson(): JSONObject = JSONObject().put("url", url).put("title", title).put("notify", notify)
        .put("added", added).put("checked", checked).put("error", error ?: JSONObject.NULL)

    companion object {
        fun fromJson(o: JSONObject) = Subscription(
            o.getString("url"), o.optString("title"), o.optBoolean("notify", true), o.optLong("added"),
            o.optLong("checked"), o.optString("error").takeIf { !o.isNull("error") && it.isNotEmpty() },
        )
    }
}

/** An article as the app keeps it: the feed's item, when Folio first saw it, and whether it was read. */
data class Article(val feedUrl: String, val feedTitle: String, val item: FeedItem, val firstSeen: Long, val read: Boolean) {
    /** newest first: the feed's date, else when it first appeared */
    val time: Long get() = item.published?.let { it * 1000 } ?: firstSeen
    val key: String get() = feedUrl + "\n" + item.id
}

private fun FeedItem.toJson(): JSONObject = JSONObject().put("id", id).put("title", title).put("link", link)
    .put("published", published ?: JSONObject.NULL).put("summary", summary).put("content", content)
    .put("author", author).put("thumb", thumb).put("minutes", minutes?.toInt() ?: JSONObject.NULL).put("needsPage", needsPage)

private fun feedItem(o: JSONObject) = FeedItem(
    id = o.getString("id"), title = o.optString("title"), link = o.optString("link"),
    published = if (o.isNull("published")) null else o.optLong("published"), summary = o.optString("summary"),
    content = o.optString("content"), author = o.optString("author"), thumb = o.optString("thumb"),
    minutes = if (o.isNull("minutes")) null else o.optInt("minutes").toUInt(), needsPage = o.optBoolean("needsPage"),
)

private fun sha1(s: String) = MessageDigest.getInstance("SHA-1").digest(s.toByteArray()).joinToString("") { "%02x".format(it) }

/**
 * Followed feeds and their recent articles, on disk. Fetching goes through [Http] (the phone's own
 * network); parsing is the shared engine's.
 */
class FeedStore private constructor(private val context: Context) {
    private val dir = File(context.filesDir, "news").apply { mkdirs() }
    private val subsFile = File(dir, "feeds.json")

    private val _subs = MutableStateFlow(loadSubs())
    val subscriptions: StateFlow<List<Subscription>> = _subs
    private val _articles = MutableStateFlow(loadAllArticles())
    val articles: StateFlow<List<Article>> = _articles

    private fun loadSubs(): List<Subscription> = try {
        val a = JSONArray(subsFile.readText())
        (0 until a.length()).map { Subscription.fromJson(a.getJSONObject(it)) }
    } catch (e: Exception) {
        emptyList()
    }

    @Synchronized
    private fun saveSubs(list: List<Subscription>) {
        _subs.value = list
        val a = JSONArray()
        list.forEach { a.put(it.toJson()) }
        writeAtomic(subsFile, a.toString())
    }

    private fun writeAtomic(f: File, text: String) {
        val tmp = File(f.parentFile, f.name + ".tmp")
        tmp.writeText(text)
        tmp.renameTo(f)
    }

    private fun itemsFile(url: String) = File(dir, sha1(url) + ".json")

    /** Per feed: recent articles, and every id seen (so old articles never notify again). */
    private class FeedData(val articles: MutableList<Article>, val seen: LinkedHashSet<String>)

    private fun loadFeed(sub: Subscription): FeedData {
        return try {
            val o = JSONObject(itemsFile(sub.url).readText())
            val items = o.getJSONArray("items")
            val seenArr = o.getJSONArray("seen")
            FeedData(
                (0 until items.length()).map {
                    val x = items.getJSONObject(it)
                    Article(sub.url, sub.title, feedItem(x.getJSONObject("item")), x.optLong("firstSeen"), x.optBoolean("read"))
                }.toMutableList(),
                LinkedHashSet((0 until seenArr.length()).map { seenArr.getString(it) }),
            )
        } catch (e: Exception) {
            FeedData(mutableListOf(), LinkedHashSet())
        }
    }

    private fun saveFeed(url: String, d: FeedData) {
        val items = JSONArray()
        d.articles.sortedByDescending { it.time }.take(MAX_PER_FEED).forEach {
            items.put(JSONObject().put("item", it.item.toJson()).put("firstSeen", it.firstSeen).put("read", it.read))
        }
        val seen = JSONArray()
        d.seen.toList().takeLast(MAX_SEEN).forEach { seen.put(it) }
        writeAtomic(itemsFile(url), JSONObject().put("items", items).put("seen", seen).toString())
    }

    // _subs is initialised before _articles, so this can read it
    private fun loadAllArticles(): List<Article> =
        _subs.value.flatMap { loadFeed(it).articles }.sortedByDescending { it.time }

    private fun publish() {
        _articles.value = _subs.value.flatMap { loadFeed(it).articles }.sortedByDescending { it.time }
    }

    /** Follow a feed from an address or a site ("scroll.in"); finds the site's feed if needed. */
    suspend fun add(text: String, name: String? = null): Result<Subscription> = runCatching {
        val address = feedAddress(text) ?: error("That doesn't look like a web address.")
        var fetched = Http.get(address)
        if (!looksLikeFeed(fetched.text)) {
            val link = findFeedLink(fetched.url, fetched.text)
                ?: error("This site doesn't advertise a feed. Look for an RSS link on it.")
            fetched = Http.get(link)
            if (!looksLikeFeed(fetched.text)) error("The site's feed link didn't return a feed.")
        }
        val parsed = parseFeed(fetched.url, fetched.text)
        _subs.value.firstOrNull { it.url == parsed.url }?.let { return@runCatching it }
        // a short name: the catalogue's, else the feed's title up to its first ":" or " - " tagline
        val title = name ?: parsed.title.split(": ", " - ", " | ", " — ").first().trim().ifBlank { parsed.title }
        val sub = Subscription(parsed.url, title, checked = System.currentTimeMillis())
        val now = System.currentTimeMillis()
        // everything already in the feed counts as seen: following a feed doesn't notify its backlog
        val d = FeedData(parsed.items.map { Article(sub.url, sub.title, it, now, false) }.toMutableList(),
                         LinkedHashSet(parsed.items.map { it.id }))
        withContext(Dispatchers.IO) { saveFeed(sub.url, d) }
        saveSubs(_subs.value + sub)
        publish()
        sub
    }.recoverCatching { throw Exception(it.friendly()) }

    fun remove(url: String) {
        itemsFile(url).delete()
        saveSubs(_subs.value.filter { it.url != url })
        publish()
    }

    fun setNotify(url: String, on: Boolean) {
        saveSubs(_subs.value.map { if (it.url == url) it.copy(notify = on) else it })
    }

    fun markRead(a: Article) = synchronized(this) {
        val sub = _subs.value.firstOrNull { it.url == a.feedUrl } ?: return@synchronized
        val d = loadFeed(sub)
        val i = d.articles.indexOfFirst { it.item.id == a.item.id }
        if (i >= 0 && !d.articles[i].read) {
            d.articles[i] = d.articles[i].copy(read = true)
            saveFeed(sub.url, d)
            _articles.value = _articles.value.map { if (it.key == a.key) it.copy(read = true) else it }
        }
    }

    fun find(feedUrl: String, id: String): Article? = _articles.value.firstOrNull { it.feedUrl == feedUrl && it.item.id == id }

    /** Fetch every feed; returns the articles not seen before (newest first). */
    suspend fun refresh(): List<Article> = coroutineScope {
        val gate = Semaphore(4)
        val results = _subs.value.map { sub ->
            async {
                gate.withPermit {
                    try {
                        val f = Http.get(sub.url)
                        val parsed = parseFeed(f.url, f.text)
                        sub to Result.success(parsed)
                    } catch (e: Exception) {
                        sub to Result.failure(e)
                    }
                }
            }
        }.awaitAll()
        val now = System.currentTimeMillis()
        val fresh = mutableListOf<Article>()
        withContext(Dispatchers.IO) {
            synchronized(this@FeedStore) {
                val updated = _subs.value.map { s ->
                    val (_, r) = results.firstOrNull { it.first.url == s.url } ?: return@map s
                    r.fold(
                        onSuccess = { parsed ->
                            val title = s.title.ifBlank { parsed.title }
                            val d = loadFeed(s)
                            val known = d.articles.associateBy { it.item.id }
                            val merged = parsed.items.map { item ->
                                known[item.id]?.copy(item = item) ?: Article(s.url, title, item, now, false).also {
                                    if (item.id !in d.seen) fresh.add(it)
                                }
                            }
                            val kept = d.articles.filter { a -> parsed.items.none { it.id == a.item.id } }
                            d.articles.clear()
                            d.articles.addAll(merged + kept)
                            parsed.items.forEach { d.seen.add(it.id) }
                            saveFeed(s.url, d)
                            s.copy(title = title, checked = now, error = null)
                        },
                        onFailure = { s.copy(checked = now, error = it.friendly()) },
                    )
                }
                saveSubs(updated)
                publish()
            }
        }
        fresh.sortedByDescending { it.time }
    }

    companion object {
        private const val MAX_PER_FEED = 60
        private const val MAX_SEEN = 2000

        @Volatile private var instance: FeedStore? = null
        fun get(context: Context): FeedStore =
            instance ?: synchronized(this) { instance ?: FeedStore(context.applicationContext).also { instance = it } }
    }
}

// --- notification settings ------------------------------------------------------------------------

enum class CheckEvery(val label: String, val minutes: Long) {
    M30("30 minutes", 30), H1("Hour", 60), H3("3 hours", 180), H6("6 hours", 360), H12("12 hours", 720), D1("Day", 1440)
}

data class NewsSettings(
    /** check feeds in the background and notify about new articles */
    val notifications: Boolean = false,
    val every: CheckEvery = CheckEvery.H1,
    val wifiOnly: Boolean = false,
    /** one summary notification instead of one per article */
    val digest: Boolean = false,
    /** deliver silently between these hours */
    val quietHours: Boolean = true,
    val quietFrom: Int = 22,
    val quietTo: Int = 7,
    /** the "get notified" suggestion was dismissed */
    val promptDismissed: Boolean = false,
) {
    fun isQuiet(hour: Int): Boolean = quietHours && quietFrom != quietTo &&
        if (quietFrom < quietTo) hour in quietFrom until quietTo else hour >= quietFrom || hour < quietTo
}

class NewsSettingsStore private constructor(context: Context) {
    private val prefs = context.getSharedPreferences("news", Context.MODE_PRIVATE)
    private val _s = MutableStateFlow(read())
    val settings: StateFlow<NewsSettings> = _s

    private fun read(): NewsSettings {
        val d = NewsSettings()
        return NewsSettings(
            notifications = prefs.getBoolean("notifications", d.notifications),
            every = CheckEvery.entries.firstOrNull { it.name == prefs.getString("every", null) } ?: d.every,
            wifiOnly = prefs.getBoolean("wifiOnly", d.wifiOnly),
            digest = prefs.getBoolean("digest", d.digest),
            quietHours = prefs.getBoolean("quietHours", d.quietHours),
            quietFrom = prefs.getInt("quietFrom", d.quietFrom),
            quietTo = prefs.getInt("quietTo", d.quietTo),
            promptDismissed = prefs.getBoolean("promptDismissed", d.promptDismissed),
        )
    }

    fun update(context: Context, f: (NewsSettings) -> NewsSettings) {
        val s = f(_s.value)
        _s.value = s
        prefs.edit().putBoolean("notifications", s.notifications).putString("every", s.every.name)
            .putBoolean("wifiOnly", s.wifiOnly).putBoolean("digest", s.digest).putBoolean("quietHours", s.quietHours)
            .putInt("quietFrom", s.quietFrom).putInt("quietTo", s.quietTo).putBoolean("promptDismissed", s.promptDismissed)
            .apply()
        FeedSchedule.apply(context, s)
    }

    companion object {
        @Volatile private var instance: NewsSettingsStore? = null
        fun get(context: Context): NewsSettingsStore =
            instance ?: synchronized(this) { instance ?: NewsSettingsStore(context.applicationContext).also { instance = it } }
    }
}

// --- background checks ------------------------------------------------------------------------------

object FeedSchedule {
    private const val WORK = "feed-check"

    /** Run one background check right away (the same work the schedule does). */
    fun checkNow(context: Context) {
        val req = androidx.work.OneTimeWorkRequestBuilder<FeedWorker>()
            .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
            .build()
        WorkManager.getInstance(context).enqueueUniqueWork("$WORK-now", androidx.work.ExistingWorkPolicy.REPLACE, req)
    }

    /** Background checks run only while notifications are on; otherwise feeds refresh when opened. */
    fun apply(context: Context, s: NewsSettings) {
        val wm = WorkManager.getInstance(context)
        if (!s.notifications) {
            wm.cancelUniqueWork(WORK)
            return
        }
        val constraints = Constraints.Builder()
            .setRequiredNetworkType(if (s.wifiOnly) NetworkType.UNMETERED else NetworkType.CONNECTED)
            .setRequiresBatteryNotLow(true)
            .build()
        val req = PeriodicWorkRequestBuilder<FeedWorker>(s.every.minutes, TimeUnit.MINUTES)
            .setConstraints(constraints)
            .build()
        wm.enqueueUniquePeriodicWork(WORK, ExistingPeriodicWorkPolicy.UPDATE, req)
    }
}

class FeedWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val settings = NewsSettingsStore.get(applicationContext).settings.value
        if (!settings.notifications) return Result.success()
        val store = FeedStore.get(applicationContext)
        val fresh = try { store.refresh() } catch (e: Exception) { return Result.retry() }
        val notifyFeeds = store.subscriptions.value.filter { it.notify }.map { it.url }.toSet()
        Notifier.announce(applicationContext, fresh.filter { it.feedUrl in notifyFeeds }, settings)
        return Result.success()
    }
}

// --- notifications ----------------------------------------------------------------------------------

object Notifier {
    const val CHANNEL = "new-articles"
    private const val GROUP = "io.folio.reader.ARTICLES"
    private const val SUMMARY_ID = 1
    private const val MAX_SINGLE = 4
    const val EXTRA_FEED = "io.folio.reader.FEED"
    const val EXTRA_ARTICLE = "io.folio.reader.ARTICLE"
    const val EXTRA_NEWS = "io.folio.reader.NEWS"

    fun ensureChannel(context: Context) {
        if (Build.VERSION.SDK_INT < 26) return
        val nm = context.getSystemService(NotificationManager::class.java)
        if (nm.getNotificationChannel(CHANNEL) == null) {
            nm.createNotificationChannel(NotificationChannel(CHANNEL, "New articles", NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = "New articles from the feeds you follow"
            })
        }
    }

    fun permitted(context: Context): Boolean = Build.VERSION.SDK_INT < 33 ||
        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    fun announce(context: Context, fresh: List<Article>, s: NewsSettings) {
        if (fresh.isEmpty() || !permitted(context)) return
        ensureChannel(context)
        val nm = NotificationManagerCompat.from(context)
        val quiet = s.isQuiet(Calendar.getInstance().get(Calendar.HOUR_OF_DAY))
        val openNews = PendingIntent.getActivity(
            context, 0, Intent(context, MainActivity::class.java).putExtra(EXTRA_NEWS, true)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val singles = if (s.digest) emptyList() else fresh.take(MAX_SINGLE)
        try {
            for (a in singles) {
                val open = PendingIntent.getActivity(
                    context, a.key.hashCode(),
                    Intent(context, MainActivity::class.java).putExtra(EXTRA_FEED, a.feedUrl).putExtra(EXTRA_ARTICLE, a.item.id)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
                    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                )
                nm.notify(a.key.hashCode(), NotificationCompat.Builder(context, CHANNEL)
                    .setSmallIcon(R.drawable.ic_notification).setColor(0xFF141414.toInt())
                    .setContentTitle(a.item.title)
                    .setContentText(a.feedTitle)
                    .setStyle(NotificationCompat.BigTextStyle().bigText(a.item.summary.ifBlank { a.feedTitle }).setSummaryText(a.feedTitle))
                    .setWhen(a.time).setShowWhen(true)
                    .setContentIntent(open).setAutoCancel(true)
                    .setGroup(GROUP).setSilent(quiet)
                    .setCategory(NotificationCompat.CATEGORY_RECOMMENDATION)
                    .build())
            }
            if (s.digest || fresh.size > singles.size || singles.size > 1) {
                val feeds = fresh.map { it.feedTitle }.distinct()
                val inbox = NotificationCompat.InboxStyle()
                fresh.take(6).forEach { inbox.addLine("${it.feedTitle}: ${it.item.title}") }
                if (fresh.size > 6) inbox.setSummaryText("+${fresh.size - 6} more")
                val title = "${fresh.size} new ${if (fresh.size == 1) "article" else "articles"}"
                nm.notify(SUMMARY_ID, NotificationCompat.Builder(context, CHANNEL)
                    .setSmallIcon(R.drawable.ic_notification).setColor(0xFF141414.toInt())
                    .setContentTitle(title)
                    .setContentText(feeds.take(3).joinToString(", ") + if (feeds.size > 3) " and more" else "")
                    .setStyle(inbox)
                    .setContentIntent(openNews).setAutoCancel(true)
                    .setGroup(GROUP).setGroupSummary(true).setSilent(quiet)
                    .build())
            }
        } catch (e: SecurityException) {
            // permission revoked between the check and the post
        }
    }
}

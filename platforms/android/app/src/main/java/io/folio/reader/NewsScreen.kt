package io.folio.reader

import android.Manifest
import android.content.Context
import android.content.Intent
import android.graphics.BitmapFactory
import android.os.Build
import android.provider.Settings
import android.text.format.DateUtils
import android.util.LruCache
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.NotificationsActive
import androidx.compose.material.icons.outlined.NotificationsOff
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Tune
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import uniffi.engine_ffi.CatalogFeed
import uniffi.engine_ffi.Position
import uniffi.engine_ffi.ReadingSession
import uniffi.engine_ffi.feedCatalog
import java.io.File
import java.security.MessageDigest

/** Read an article: fetch its page when the feed holds only a teaser, falling back to the feed's text. */
fun articleSource(store: FeedStore, a: Article): ReadingSource =
    ReadingSource(a.item.title, a.feedTitle, ArticleState) { fonts ->
        store.markRead(a)
        val page = if (a.item.needsPage) try { Http.get(a.item.link).text } catch (e: Exception) { null } else null
        try {
            ReadingSession.openArticle(a.item, page, fonts)
        } catch (e: Exception) {
            if (page == null) throw e
            ReadingSession.openArticle(a.item, null, fonts)
        }
    }

/** Articles open at the top each time and have no bookmarks. */
object ArticleState : ReadingState {
    override fun position(): Position? = null
    override fun save(position: Position, progress: Float) {}
    override fun bookmarks(): List<Bookmark> = emptyList()
    override fun saveBookmarks(list: List<Bookmark>) {}
    override val supportsBookmarks get() = false
}

/** News has the list, and one page to manage it: feeds and alerts together. */
enum class NewsPage { List, Manage, Follow }

@Composable
fun NewsScreen(onOpen: (Article) -> Unit, onSettings: () -> Unit, bottomBar: @Composable () -> Unit) {
    val context = LocalContext.current
    val store = remember { FeedStore.get(context) }
    val settingsStore = remember { NewsSettingsStore.get(context) }
    var page by rememberSaveable { mutableStateOf(NewsPage.List) }
    when (page) {
        NewsPage.List -> ArticleList(store, settingsStore, onOpen, { page = it }, onSettings, bottomBar)
        NewsPage.Manage -> ManagePage(store, settingsStore, onFollow = { page = NewsPage.Follow }) { page = NewsPage.List }
        NewsPage.Follow -> FollowPage(store) { page = NewsPage.Manage }
    }
    if (page != NewsPage.List) androidx.activity.compose.BackHandler { page = if (page == NewsPage.Follow) NewsPage.Manage else NewsPage.List }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ArticleList(
    store: FeedStore,
    settingsStore: NewsSettingsStore,
    onOpen: (Article) -> Unit,
    go: (NewsPage) -> Unit,
    onSettings: () -> Unit,
    bottomBar: @Composable () -> Unit,
) {
    val l = look
    val context = LocalContext.current
    val subs by store.subscriptions.collectAsState()
    val articles by store.articles.collectAsState()
    val settings by settingsStore.settings.collectAsState()
    val scope = rememberCoroutineScope()
    var refreshing by remember { mutableStateOf(false) }
    var feedFilter by rememberSaveable { mutableStateOf<String?>(null) }
    var message by remember { mutableStateOf<String?>(null) }
    fun refresh() {
        if (refreshing || subs.isEmpty()) return
        refreshing = true
        scope.launch {
            store.refresh()
            refreshing = false
            val failed = store.subscriptions.value.count { it.error != null }
            message = if (failed > 0) "$failed ${if (failed == 1) "feed" else "feeds"} couldn't be updated" else null
        }
    }
    // fresh news when the screen opens, if the last check is more than 15 minutes old
    LaunchedEffect(subs.size) {
        val last = subs.maxOfOrNull { it.checked } ?: 0
        if (System.currentTimeMillis() - last > 15 * 60_000) refresh()
    }
    Scaffold(
        containerColor = l.p.paper,
        topBar = {
            Header(listOf("News")) {
                if (subs.isNotEmpty()) HeaderIcon(Icons.Outlined.Tune, "Feeds and alerts", { go(NewsPage.Manage) })
                HeaderIcon(Icons.Outlined.Settings, "Settings", onSettings)
            }
        },
        bottomBar = bottomBar,
    ) { pad ->
        if (subs.isEmpty()) {
            FollowFirst(store, Modifier.padding(pad))
            return@Scaffold
        }
        PullToRefreshBox(isRefreshing = refreshing, onRefresh = ::refresh, modifier = Modifier.padding(pad).fillMaxSize()) {
            val shown = articles.filter { feedFilter == null || it.feedUrl == feedFilter }
            LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 24.dp)) {
                if (!settings.notifications && !settings.promptDismissed) {
                    item(key = "prompt") {
                        NotifyBanner(
                            onEnable = { go(NewsPage.Manage) },
                            onDismiss = { settingsStore.update(context) { it.copy(promptDismissed = true) } },
                        )
                    }
                }
                item(key = "bar") {
                    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 2.dp), verticalAlignment = Alignment.CenterVertically) {
                        if (subs.size > 1) {
                            Picker(feedFilter, listOf<String?>(null) + subs.map { it.url },
                                   { u -> if (u == null) "All feeds" else subs.firstOrNull { it.url == u }?.title ?: "" },
                                   { feedFilter = it })
                        }
                        Spacer(Modifier.weight(1f))
                        Text(l.caps(if (refreshing) "Updating…" else "${shown.count { !it.read }} unread"), style = Type.label, color = l.p.muted)
                    }
                }
                message?.let { m ->
                    item(key = "msg") {
                        Text(m, style = Type.meta, color = l.p.accent,
                             modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp).clickable { go(NewsPage.Manage) })
                    }
                }
                if (shown.isEmpty()) {
                    item(key = "empty") {
                        Box(Modifier.fillMaxWidth().padding(32.dp)) {
                            if (refreshing) Working("Fetching") else Text("No articles yet. Pull down to refresh.", style = Type.body, color = l.p.muted)
                        }
                    }
                }
                items(shown, key = { it.key }) { a ->
                    ArticleRow(a) { onOpen(a) }
                    Rule(Modifier.padding(horizontal = 16.dp), color = l.p.faint)
                }
            }
        }
    }
}

/** A one-line offer to turn alerts on. */
@Composable
private fun NotifyBanner(onEnable: () -> Unit, onDismiss: () -> Unit) {
    val l = look
    Row(
        Modifier.fillMaxWidth().background(l.p.well).clickable(onClick = onEnable).padding(start = 16.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text("Get an alert when new articles are published", style = Type.meta, color = l.p.ink, modifier = Modifier.weight(1f))
        Text(l.caps("Turn on"), style = Type.button, color = l.p.accent, modifier = Modifier.padding(horizontal = 8.dp))
        Text("×", style = Type.ui, color = l.p.muted, modifier = Modifier.clickable(onClick = onDismiss).padding(horizontal = 12.dp, vertical = 8.dp))
    }
}

@Composable
private fun ArticleRow(a: Article, onClick: () -> Unit) {
    val l = look
    Row(Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 14.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(a.item.title, style = Type.title.copy(fontSize = 17.sp, lineHeight = 22.sp, fontWeight = if (a.read) FontWeight.Normal else FontWeight.SemiBold),
                 maxLines = 3, overflow = TextOverflow.Ellipsis, color = if (a.read) l.p.muted else l.p.ink)
            Spacer(Modifier.height(5.dp))
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (!a.read) {
                    Box(Modifier.size(6.dp).background(l.p.accent, if (l.theme.corner > 0.dp) androidx.compose.foundation.shape.CircleShape else RectangleShape))
                    Spacer(Modifier.width(6.dp))
                }
                Text(a.feedTitle, style = Type.meta, color = l.p.muted, maxLines = 1, overflow = TextOverflow.Ellipsis,
                     modifier = Modifier.weight(1f, fill = false))
                Text(" · " + DateUtils.getRelativeTimeSpanString(a.time, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS,
                                                                 DateUtils.FORMAT_ABBREV_RELATIVE),
                     style = Type.meta, color = l.p.muted, maxLines = 1)
            }
        }
        if (a.item.thumb.startsWith("http")) {
            Spacer(Modifier.width(14.dp))
            Box(Modifier.size(72.dp).clip(l.smallShape)) { RemoteImage(a.item.thumb, Modifier.fillMaxSize()) }
        }
    }
}

private val thumbMemory = LruCache<String, ImageBitmap>(80)
private val thumbGate = Semaphore(4)

/** A picture from the web (article thumbnails, book covers): memory, then disk cache, then the network. */
@Composable
fun RemoteImage(url: String, modifier: Modifier) {
    val context = LocalContext.current
    val l = look
    val img by produceState(thumbMemory.get(url), url) {
        if (value != null) return@produceState
        value = withContext(Dispatchers.IO) {
            val dir = File(context.cacheDir, "thumbs").apply { mkdirs() }
            val f = File(dir, MessageDigest.getInstance("SHA-1").digest(url.toByteArray()).joinToString("") { "%02x".format(it) })
            try {
                if (!f.exists()) thumbGate.withPermit { f.writeBytes(Http.get(url, maxBytes = 4 shl 20).bytes) }
                val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeFile(f.path, bounds)
                var sample = 1
                while (minOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= 240) sample *= 2
                BitmapFactory.decodeFile(f.path, BitmapFactory.Options().apply { inSampleSize = sample })?.asImageBitmap()
                    ?.also { thumbMemory.put(url, it) }
            } catch (e: Exception) {
                null
            }
        }
    }
    Box(modifier.background(l.p.well)) {
        img?.let { Image(it, contentDescription = null, contentScale = ContentScale.Crop, colorFilter = l.imageFilter, modifier = Modifier.fillMaxSize()) }
    }
}

// --- following feeds ------------------------------------------------------------------------------------

@Composable
private fun FollowFirst(store: FeedStore, modifier: Modifier) {
    val l = look
    Column(modifier.fillMaxSize()) {
        Column(Modifier.padding(start = 16.dp, end = 16.dp, top = 20.dp, bottom = 4.dp)) {
            Text(l.caps("Follow the news you read"), style = Type.display, color = l.p.ink)
            Spacer(Modifier.height(8.dp))
            Text("Pick some feeds, or add any website that has one. Articles open in the reader, without ads or clutter.",
                 style = Type.body.copy(fontSize = 15.sp, lineHeight = 21.sp), color = l.p.muted)
        }
        AddFeed(store, Modifier.weight(1f))
    }
}

/** Add by address, or browse the catalogue. */
@Composable
private fun AddFeed(store: FeedStore, modifier: Modifier, onAdded: () -> Unit = {}) {
    val l = look
    val subs by store.subscriptions.collectAsState()
    val scope = rememberCoroutineScope()
    var address by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val catalog = remember { feedCatalog() }
    val categories = remember { listOf<String?>(null) + catalog.map { it.category }.distinct() }
    var category by rememberSaveable { mutableStateOf<String?>(null) }
    fun add(text: String, name: String? = null) {
        busy = text; error = null
        scope.launch {
            store.add(text, name).onSuccess { address = ""; onAdded() }.onFailure { error = it.message }
            busy = null
        }
    }
    LazyColumn(modifier.fillMaxWidth(), contentPadding = PaddingValues(bottom = 24.dp)) {
        item {
            InkField(
                value = address, onChange = { address = it; error = null }, placeholder = "Website or feed, e.g. scroll.in",
                prompt = if (l.theme.id == "eink") "+" else "",
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go),
                onSubmit = { if (address.isNotBlank()) add(address) },
                trailing = {
                    if (busy == address && address.isNotBlank()) Working("", Modifier.padding(horizontal = 10.dp))
                    else if (address.isNotBlank()) InkButton("Follow", { add(address) }, compact = true, filled = true)
                },
                modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp),
            )
            error?.let { Text(it, style = Type.meta, color = l.p.accent, modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp)) }
        }
        item {
            Spacer(Modifier.height(12.dp))
            Box(Modifier.padding(horizontal = 16.dp)) {
                Segmented(categories, category, { it ?: "All" }, scroll = true) { category = it }
            }
            Spacer(Modifier.height(6.dp))
        }
        items(catalog.filter { category == null || it.category == category }, key = { it.url }) { f ->
            CatalogRow(f, following = subs.any { it.url == f.url }, busy = busy == f.url) { add(f.url, f.name) }
            Rule(Modifier.padding(horizontal = 16.dp), color = l.p.faint)
        }
    }
}

@Composable
private fun CatalogRow(f: CatalogFeed, following: Boolean, busy: Boolean, onAdd: () -> Unit) {
    val l = look
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(f.name, style = Type.title.copy(fontSize = 16.sp), color = l.p.ink)
            Text(f.description, style = Type.meta, color = l.p.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        Spacer(Modifier.width(10.dp))
        when {
            following -> Tag("Following", inverted = true)
            busy -> Working("")
            else -> InkButton("Follow", onAdd, compact = true)
        }
    }
}

/** Adding a feed: by address, or from the catalogue. */
@Composable
private fun FollowPage(store: FeedStore, onBack: () -> Unit) {
    val l = look
    Scaffold(containerColor = l.p.paper, topBar = { Header(listOf("News", "Follow a feed"), onBack = onBack) }) { pad ->
        AddFeed(store, Modifier.padding(pad)) { onBack() }
    }
}

/** Feeds and alerts in one place. */
@Composable
private fun ManagePage(store: FeedStore, settingsStore: NewsSettingsStore, onFollow: () -> Unit, onBack: () -> Unit) {
    val l = look
    val context = LocalContext.current
    val s by settingsStore.settings.collectAsState()
    val subs by store.subscriptions.collectAsState()
    var removing by remember { mutableStateOf<Subscription?>(null) }
    var denied by remember { mutableStateOf(false) }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) settingsStore.update(context) { it.copy(notifications = true) } else denied = true
    }
    fun setEnabled(on: Boolean) {
        if (on && !Notifier.permitted(context) && Build.VERSION.SDK_INT >= 33) {
            permission.launch(Manifest.permission.POST_NOTIFICATIONS)
        } else {
            settingsStore.update(context) { it.copy(notifications = on) }
        }
    }
    val on = s.notifications
    Scaffold(containerColor = l.p.paper, topBar = { Header(listOf("News", "Feeds & alerts"), onBack = onBack) }) { pad ->
        Column(Modifier.padding(pad).fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 32.dp)) {
            SectionLabel("Feeds", trailing = "${subs.size}")
            subs.forEach { sub ->
                Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp, top = 6.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(sub.title, style = Type.ui, color = l.p.ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(sub.error ?: sub.url.substringAfter("://").substringBefore('/').removePrefix("www."), style = Type.meta,
                             color = if (sub.error != null) l.p.accent else l.p.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    if (on) {
                        HeaderIcon(if (sub.notify) Icons.Outlined.NotificationsActive else Icons.Outlined.NotificationsOff,
                                   if (sub.notify) "Alerts on for ${sub.title}" else "Alerts off for ${sub.title}",
                                   { store.setNotify(sub.url, !sub.notify) }, highlighted = sub.notify)
                    }
                    HeaderIcon(Icons.Outlined.Close, "Unfollow ${sub.title}", { removing = sub })
                }
            }
            Box(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) { InkButton("+ Follow a feed", onFollow) }

            SectionLabel("Alerts", Modifier.padding(top = 12.dp))
            Column(Modifier.padding(horizontal = 16.dp)) {
                CheckRow("Notify me about new articles", "Folio checks your feeds in the background", on) { setEnabled(it) }
                if (denied) {
                    Text("Notifications are blocked for Folio. Allow them in Android's settings.", style = Type.meta, color = l.p.accent)
                    Spacer(Modifier.height(8.dp))
                    InkButton("Open Android settings", { openSystemSettings(context) }, compact = true)
                }
                if (on) {
                    Row(Modifier.fillMaxWidth().padding(vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text("Check every", style = Type.ui, color = l.p.ink, modifier = Modifier.weight(1f))
                        Picker(s.every, CheckEvery.entries, { it.label }, { v -> settingsStore.update(context) { it.copy(every = v) } })
                    }
                    CheckRow("Only on Wi-Fi", null, s.wifiOnly) { v -> settingsStore.update(context) { it.copy(wifiOnly = v) } }
                    CheckRow("One summary instead of one per article", null, s.digest) { v -> settingsStore.update(context) { it.copy(digest = v) } }
                    CheckRow("Silent at night", "No sound or vibration from ${hour(s.quietFrom)} to ${hour(s.quietTo)}", s.quietHours) { v ->
                        settingsStore.update(context) { it.copy(quietHours = v) }
                    }
                    if (s.quietHours) {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                            Text("From", style = Type.meta, color = l.p.muted)
                            Picker(s.quietFrom, (0..23).toList(), ::hour, { v -> settingsStore.update(context) { it.copy(quietFrom = v) } })
                            Text("to", style = Type.meta, color = l.p.muted)
                            Picker(s.quietTo, (0..23).toList(), ::hour, { v -> settingsStore.update(context) { it.copy(quietTo = v) } })
                        }
                    }
                    Spacer(Modifier.height(12.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        InkButton("Check now", {
                            FeedSchedule.checkNow(context)
                            android.widget.Toast.makeText(context, "Checking your feeds…", android.widget.Toast.LENGTH_SHORT).show()
                        }, compact = true)
                        InkButton("Sound & vibration", { openSystemSettings(context) }, compact = true)
                    }
                }
            }
        }
    }
    removing?.let { sub ->
        InkDialog(
            title = "Unfollow", text = "Stop following ${sub.title}? Its articles will be removed from News.",
            confirm = "Unfollow", onConfirm = { store.remove(sub.url); removing = null }, onDismiss = { removing = null },
        )
    }
}

private fun hour(h: Int): String {
    val cal = java.util.Calendar.getInstance().apply { set(java.util.Calendar.HOUR_OF_DAY, h); set(java.util.Calendar.MINUTE, 0) }
    return java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT).format(cal.time)
}

private fun openSystemSettings(context: Context) {
    val i = if (Build.VERSION.SDK_INT >= 26) {
        Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
    } else {
        Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, android.net.Uri.parse("package:" + context.packageName))
    }
    context.startActivity(i)
}

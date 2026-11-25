package io.folio.reader

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.view.KeyEvent
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private lateinit var library: Library
    private lateinit var settings: SettingsStore

    /** What is open: "book:<id>", "article:<feed url>\n<item id>", or null for the tabs. */
    private var reading by mutableStateOf<String?>(null)
    private var tab by mutableStateOf(Tab.Library)
    private var settingsOpen by mutableStateOf(false)

    /** Page-turn requests from the volume keys, for the reader screen. */
    val volumeTurns = MutableSharedFlow<Int>(extraBufferCapacity = 4)

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        FontBank.init(this)
        CatalogCache.init(this)
        library = Library.get(this)
        settings = SettingsStore(applicationContext)
        reading = savedInstanceState?.getString("reading")
        tab = savedInstanceState?.getString("tab")?.let { t -> Tab.entries.firstOrNull { it.name == t } } ?: Tab.Library
        if (savedInstanceState == null) handleIntent(intent)
        Notifier.ensureChannel(this)
        FeedSchedule.apply(this, NewsSettingsStore.get(this).settings.value)
        val feeds = FeedStore.get(this)
        setContent {
            FolioTheme {
                val r = reading
                val source = remember(r) {
                    when {
                        r == null -> null
                        r.startsWith("book:") -> library.get(r.removePrefix("book:"))?.let { ReadingSource.book(library, it) }
                        r.startsWith("article:") -> r.removePrefix("article:").split("\n", limit = 2).let { (f, id) ->
                            feeds.find(f, id)?.let { articleSource(feeds, it) }
                        }
                        else -> null
                    }
                }
                if (source == null && settingsOpen) {
                    SettingsScreen(onBack = { settingsOpen = false })
                } else if (source == null) {
                    val bar: @Composable () -> Unit = {
                        TabBar(Tab.entries.map { it.label }, tab.ordinal) { tab = Tab.entries[it] }
                    }
                    when (tab) {
                        Tab.Library -> LibraryScreen(library, onOpen = { reading = "book:" + it.id }, onImport = ::importAll,
                                                     onFindBooks = { tab = Tab.GetBooks }, onSettings = { settingsOpen = true },
                                                     bottomBar = bar)
                        Tab.GetBooks -> GetBooksScreen(onRead = { reading = "book:$it" }, onSettings = { settingsOpen = true }, bottomBar = bar)
                        Tab.News -> NewsScreen(onOpen = { reading = "article:" + it.feedUrl + "\n" + it.item.id }, onSettings = { settingsOpen = true }, bottomBar = bar)
                    }
                } else {
                    BackHandler { reading = null }
                    ReaderScreen(settings = settings, source = source, key = r!!, volumeTurns = volumeTurns, onClose = { reading = null })
                }
            }
        }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putString("reading", reading)
        outState.putString("tab", tab.name)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    /** "Open with Folio", "Share to Folio", or a tapped notification. */
    private fun handleIntent(intent: Intent?) {
        if (intent?.getBooleanExtra(Notifier.EXTRA_NEWS, false) == true) {
            reading = null; tab = Tab.News; return
        }
        val feed = intent?.getStringExtra(Notifier.EXTRA_FEED)
        val article = intent?.getStringExtra(Notifier.EXTRA_ARTICLE)
        if (feed != null && article != null) {
            tab = Tab.News; reading = "article:$feed\n$article"; return
        }
        val uri: Uri? = when (intent?.action) {
            Intent.ACTION_VIEW -> intent.data
            Intent.ACTION_SEND -> if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
                                  else @Suppress("DEPRECATION") intent.getParcelableExtra(Intent.EXTRA_STREAM)
            else -> null
        }
        if (uri != null) importAll(listOf(uri), open = true)
    }

    private fun importAll(uris: List<Uri>, open: Boolean = uris.size == 1) {
        lifecycleScope.launch {
            var last: Book? = null
            for (u in uris) {
                library.import(u).onSuccess { last = it }.onFailure {
                    Toast.makeText(this@MainActivity, it.message ?: "That file couldn't be added.", Toast.LENGTH_LONG).show()
                }
            }
            if (open) last?.let { tab = Tab.Library; reading = "book:" + it.id }
        }
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        val turn = when (event.keyCode) {
            KeyEvent.KEYCODE_VOLUME_DOWN, KeyEvent.KEYCODE_PAGE_DOWN -> 1
            KeyEvent.KEYCODE_VOLUME_UP, KeyEvent.KEYCODE_PAGE_UP -> -1
            else -> 0
        }
        val volume = event.keyCode == KeyEvent.KEYCODE_VOLUME_DOWN || event.keyCode == KeyEvent.KEYCODE_VOLUME_UP
        if (turn != 0 && reading != null && (!volume || settings.settings.value.volumeKeys)) {
            if (event.action == KeyEvent.ACTION_DOWN) volumeTurns.tryEmit(turn)
            return true
        }
        return super.dispatchKeyEvent(event)
    }
}

enum class Tab(val label: String) { Library("Library"), GetBooks("Get Books"), News("News") }

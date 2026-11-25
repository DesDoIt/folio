package io.folio.reader

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import uniffi.engine_ffi.FoundBook
import uniffi.engine_ffi.mergeResults

/** A book and where it was found. */
private data class Hit(val source: BookSource, val book: FoundBook)

/** A short row of books from one source, for browsing all sources at once. */
private data class Shelf(val source: BookSource, val title: String, val books: List<FoundBook>, val error: String?)

private sealed interface Load {
    data object Loading : Load
    data class Failed(val message: String) : Load
    /** browsing all sources: a shelf from each */
    data class Shelves(val shelves: List<Shelf>) : Load
    /** results: from every source (a search), or from one (with paging and folders) */
    data class Results(
        val hits: List<Hit>,
        val page: SourcePage? = null,
        /** sources that didn't answer, by name */
        val missing: List<String> = emptyList(),
        /** sources still being searched (results show as each one answers) */
        val pending: List<String> = emptyList(),
        val loadingMore: Boolean = false,
    ) : Load
}

private const val SHELF_SIZE = 5

/** How long one source may take when searching all of them; a slow one is reported, not waited for. */
private const val SOURCE_DEADLINE_MS = 12_000L

/** where source requests run, so a search can stop waiting for one without blocking on it */
private val detached = kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.SupervisorJob() + kotlinx.coroutines.Dispatchers.IO)

/**
 * Search every source at once, showing results as each source answers (mixed by the shared engine:
 * alternating, Standard Ebooks first, duplicates dropped).
 */
private suspend fun searchAll(sources: List<BookSource>, query: String, show: (Load) -> Unit) = coroutineScope {
    val found = LinkedHashMap<BookSource, List<FoundBook>>()
    val missing = mutableListOf<String>()
    val pending = sources.toMutableList()
    fun publish() {
        val lists = sources.filter { it in found }.map { it to found.getValue(it) }
        val merged = mergeResults(lists.map { it.second }, query).map { Hit(lists[it.source.toInt()].first, it.book) }
        show(
            if (merged.isEmpty() && pending.isEmpty() && missing.isNotEmpty() && found.isEmpty())
                Load.Failed("No source answered. Check your connection and try again.")
            else Load.Results(merged, missing = missing.toList(), pending = pending.map { it.label })
        )
    }
    publish()
    sources.forEach { s ->
        launch {
            // a blocking read can't be interrupted, so stop waiting rather than wait for it to give up
            val work = detached.async { runCatching { Sources.open(s, query).books } }
            val r = withTimeoutOrNull(SOURCE_DEADLINE_MS) { work.await() }
            if (r == null) work.cancel()
            when {
                // a catalogue kept on the phone is still downloading (it carries on in the background)
                r == null && (s == BookSource.GutenbergAustralia || s == BookSource.FadedPage) ->
                    missing.add("${s.label} (still fetching its book list; try again shortly)")
                r == null -> missing.add(s.label)
                r.isSuccess -> found[s] = r.getOrThrow()
                // a catalog that can't be searched just sits this one out
                s is BookSource.Opds && r.exceptionOrNull()?.message?.contains("search") == true -> {}
                else -> missing.add(s.label)
            }
            pending.remove(s)
            publish()
        }
    }
}

/** Browse everything: the newest Standard Ebooks and the most popular Gutenberg books. */
private suspend fun browseAll(): Load = coroutineScope {
    val shelves = listOf(BookSource.StandardEbooks to "Newest from Standard Ebooks", BookSource.Gutenberg to "Popular on Project Gutenberg")
        .map { (s, title) ->
            async {
                val work = detached.async { runCatching { Sources.open(s, "").books } }
                (withTimeoutOrNull(SOURCE_DEADLINE_MS) { work.await() }
                    ?: Result.failure<List<FoundBook>>(Exception("It took too long to answer.")).also { work.cancel() }).fold(
                    onSuccess = { Shelf(s, title, it.take(SHELF_SIZE), null) },
                    onFailure = { Shelf(s, title, emptyList(), it.friendly()) },
                )
            }
        }.awaitAll()
    if (shelves.all { it.error != null }) Load.Failed(shelves.first().error ?: "No source answered.") else Load.Shelves(shelves)
}

@Composable
fun GetBooksScreen(onRead: (String) -> Unit, onSettings: () -> Unit, bottomBar: @Composable () -> Unit) {
    val l = look
    val context = LocalContext.current
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    val catalogStore = remember { CatalogStore.get(context) }
    val catalogs by catalogStore.catalogs.collectAsState()
    val sourceStore = remember { SourceStore.get(context) }
    val sourceSettings by sourceStore.settings.collectAsState()
    val sources: List<BookSource> =
        listOf(BookSource.StandardEbooks, BookSource.Gutenberg) + sourceSettings.extra() + catalogs.map { BookSource.Opds(it) }
    var managingSources by remember { mutableStateOf(false) }
    /** the source filter: null = all sources */
    var filterUrl by rememberSaveable { mutableStateOf<String?>(null) }
    val filter: BookSource? = filterUrl?.let { key -> sources.firstOrNull { sourceKey(it) == key } }
    var query by rememberSaveable { mutableStateOf("") }
    var searched by rememberSaveable { mutableStateOf("") }
    /** catalog folders opened, as (title, address) */
    var stack by remember { mutableStateOf<List<Pair<String, String>>>(emptyList()) }
    var load by remember { mutableStateOf<Load>(Load.Loading) }
    var attempt by remember { mutableIntStateOf(0) }
    var addingCatalog by remember { mutableStateOf(false) }
    var removing by remember { mutableStateOf<Catalog?>(null) }
    val scope = rememberCoroutineScope()
    val listState = rememberLazyListState()
    fun setFilter(s: BookSource?) { filterUrl = s?.let(::sourceKey); stack = emptyList() }

    LaunchedEffect(filter, searched, stack, attempt, catalogs.size, sourceSettings) {
        load = Load.Loading
        load = try {
            when {
                filter == null && searched.isEmpty() -> browseAll()
                // Wikisource and the Internet Archive need a search; browsing shows only the others
                filter == null -> { searchAll(sources, searched) { load = it }; load }
                else -> {
                    val p = stack.lastOrNull()?.let { Sources.more(filter, it.second) } ?: Sources.open(filter, searched)
                    // closest matches first (and one copy of each book)
                    val books = if (searched.isEmpty()) p.books.distinctBy { Downloads.key(it) }
                                else mergeResults(listOf(p.books), searched).map { it.book }
                    Load.Results(books.map { Hit(filter, it) }, page = p)
                }
            }
        } catch (e: kotlinx.coroutines.CancellationException) {
            throw e // a newer search replaced this one
        } catch (e: Exception) {
            Load.Failed(e.friendly())
        }
        listState.scrollToItem(0)
    }
    if (stack.isNotEmpty()) BackHandler { stack = stack.dropLast(1) }
    else if (filter != null || searched.isNotEmpty()) BackHandler { setFilter(null); query = ""; searched = "" }

    fun loadMore() {
        val r = load as? Load.Results ?: return
        val src = filter ?: return
        val next = r.page?.next ?: return
        if (r.loadingMore) return
        load = r.copy(loadingMore = true)
        scope.launch {
            load = try {
                val p = Sources.more(src, next)
                val books = (r.hits.map { it.book } + p.books).distinctBy { Downloads.key(it) }
                Load.Results(books.map { Hit(src, it) }, page = p.copy(folders = r.page.folders + p.folders))
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                r.copy(loadingMore = false).also {
                    android.widget.Toast.makeText(context, e.friendly(), android.widget.Toast.LENGTH_SHORT).show()
                }
            }
        }
    }

    Scaffold(
        containerColor = l.p.paper,
        topBar = { Header(listOf("Get Books")) { HeaderIcon(Icons.Outlined.Settings, "Settings", onSettings) } },
        bottomBar = bottomBar,
    ) { pad ->
        LazyColumn(Modifier.padding(pad).fillMaxSize(), state = listState, contentPadding = PaddingValues(bottom = 24.dp)) {
            item(key = "search") {
                InkField(
                    value = query, onChange = { query = it },
                    placeholder = if (filter == null) "Search free books" else "Search ${filter.label}",
                    prompt = if (l.theme.id == "eink") "›" else "",
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                    onSubmit = { searched = query.trim(); stack = emptyList(); keyboard?.hide(); focus.clearFocus() },
                    trailing = {
                        if (query.isNotEmpty()) {
                            Text("×", style = Type.display.copy(fontSize = 20.sp), color = l.p.ink,
                                 modifier = Modifier.clickable { query = ""; searched = ""; stack = emptyList() }.padding(horizontal = 12.dp))
                        }
                    },
                    modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 14.dp, bottom = 10.dp),
                )
            }
            item(key = "filter") {
                // the source is a filter on the search, not a place you have to pick first
                val opds = filter as? BookSource.Opds
                Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Picker<BookSource?>(filter, listOf<BookSource?>(null) + sources, { it?.label ?: "All sources" }, { setFilter(it) },
                           prefix = "In",
                           extras = listOfNotNull("Book sources…" to { managingSources = true },
                                                  "Add a catalog…" to { addingCatalog = true },
                                                  opds?.let { "Remove ${it.catalog.name}" to { removing = it.catalog } }))
                    Spacer(Modifier.width(12.dp))
                    val what = when {
                        stack.isNotEmpty() -> null
                        searched.isNotEmpty() -> "“$searched”"
                        filter == BookSource.StandardEbooks -> "Newest"
                        filter == BookSource.Gutenberg -> "Most popular"
                        else -> null
                    }
                    what?.let { Text(l.caps(it), style = Type.label, color = l.p.muted, maxLines = 1, overflow = TextOverflow.Ellipsis,
                                     textAlign = TextAlign.End, modifier = Modifier.weight(1f)) }
                }
                Spacer(Modifier.height(4.dp))
            }
            if (stack.isNotEmpty()) {
                item(key = "crumb") {
                    Row(Modifier.fillMaxWidth().clickable { stack = stack.dropLast(1) }.padding(horizontal = 16.dp, vertical = 10.dp),
                        verticalAlignment = Alignment.CenterVertically) {
                        Text("‹  ", style = Type.heading, color = l.p.accent)
                        Text(l.caps(stack.last().first), style = Type.heading, color = l.p.ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    Rule(color = if (l.theme.id == "eink") l.p.ink else l.p.faint)
                }
            }
            when (val ld = load) {
                Load.Loading -> item(key = "loading") {
                    Box(Modifier.fillMaxWidth().padding(48.dp), contentAlignment = Alignment.Center) {
                        Working(when {
                            filter == null && searched.isNotEmpty() -> "Searching all sources"
                            filter == BookSource.GutenbergAustralia || filter == BookSource.FadedPage -> "Fetching the book list"
                            else -> "Loading"
                        })
                    }
                }
                is Load.Failed -> item(key = "failed") {
                    Column(Modifier.fillMaxWidth().padding(24.dp)) {
                        Text(ld.message, style = Type.body.copy(fontSize = 15.sp), color = l.p.ink)
                        Spacer(Modifier.height(12.dp))
                        InkButton("Try again", { attempt++ })
                    }
                }
                is Load.Shelves -> {
                    ld.shelves.forEach { shelf ->
                        item(key = "shelf:" + shelf.title) {
                            Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 14.dp), verticalAlignment = Alignment.CenterVertically) {
                                Text(l.caps(shelf.title), style = Type.label, color = if (l.theme.id == "editorial") l.p.accent else l.p.muted,
                                     modifier = Modifier.weight(1f))
                                if (shelf.error == null) {
                                    Text(l.caps("See all ›"), style = Type.button, color = l.p.accent,
                                         modifier = Modifier.clickable { setFilter(shelf.source) }.padding(8.dp))
                                }
                            }
                            shelf.error?.let {
                                Text("Not available right now: $it", style = Type.meta, color = l.p.muted, modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp))
                            }
                        }
                        items(shelf.books, key = { "s:" + shelf.title + Downloads.key(it) }) { b ->
                            FoundRow(b, null, null, onRead)
                            Rule(Modifier.padding(horizontal = 16.dp), color = l.p.faint)
                        }
                    }
                    if (catalogs.isNotEmpty()) {
                        item(key = "catalogs") {
                            Text(l.caps("Your catalogs"), style = Type.label, color = if (l.theme.id == "editorial") l.p.accent else l.p.muted,
                                 modifier = Modifier.padding(start = 16.dp, top = 22.dp, bottom = 4.dp))
                        }
                        items(catalogs, key = { "c:" + it.url }) { c ->
                            Row(Modifier.fillMaxWidth().clickable { setFilter(BookSource.Opds(c)) }.padding(horizontal = 16.dp, vertical = 14.dp),
                                verticalAlignment = Alignment.CenterVertically) {
                                Column(Modifier.weight(1f)) {
                                    Text(c.name, style = Type.title, color = l.p.ink)
                                    Text(c.url.substringAfter("://"), style = Type.meta, color = l.p.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                }
                                Text(l.caps("Browse ›"), style = Type.button, color = l.p.accent)
                            }
                            Rule(Modifier.padding(horizontal = 16.dp), color = l.p.faint)
                        }
                    }
                }
                is Load.Results -> {
                    if (ld.pending.isNotEmpty()) {
                        item(key = "pending") {
                            Box(Modifier.padding(horizontal = 16.dp, vertical = 10.dp)) {
                                Working("Searching ${ld.pending.joinToString(", ")}")
                            }
                        }
                    }
                    if (ld.missing.isNotEmpty()) {
                        item(key = "missing") {
                            Text("${ld.missing.joinToString(" and ")} didn't answer; showing the rest.", style = Type.meta, color = l.p.muted,
                                 modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp))
                        }
                    }
                    val folders = ld.page?.folders.orEmpty()
                    // catalogs may list the same address twice (e.g. "Recent" and "By author")
                    itemsIndexed(folders, key = { i, f -> "f:$i:" + f.second }) { _, (title, address) ->
                        Row(Modifier.fillMaxWidth().clickable { stack = stack + (title to address) }.padding(horizontal = 16.dp, vertical = 16.dp),
                            verticalAlignment = Alignment.CenterVertically) {
                            Text(title, style = Type.title, color = l.p.ink, modifier = Modifier.weight(1f))
                            Text("›", style = Type.display.copy(fontSize = 20.sp), color = l.p.muted)
                        }
                        Rule(Modifier.padding(horizontal = 16.dp), color = l.p.faint)
                    }
                    if (ld.hits.isEmpty() && folders.isEmpty() && ld.pending.isEmpty()) {
                        item(key = "none") {
                            Text(ld.page?.hint ?: if (searched.isNotEmpty()) "No books found for “$searched”." else "Nothing here.",
                                 style = Type.body.copy(fontSize = 15.sp), color = l.p.muted, modifier = Modifier.padding(24.dp))
                        }
                    }
                    items(ld.hits, key = { "b:" + Downloads.key(it.book) }) { h ->
                        FoundRow(h.book, if (filter == null) h.source.label else null, (h.source as? BookSource.Opds)?.catalog?.login, onRead)
                        Rule(Modifier.padding(horizontal = 16.dp), color = l.p.faint)
                    }
                    if (filter != null && ld.page?.next != null) {
                        item(key = "more") {
                            LaunchedEffect(ld.hits.size) { loadMore() } // reaching the end loads the next page
                            Box(Modifier.fillMaxWidth().padding(20.dp), contentAlignment = Alignment.Center) { Working("More") }
                        }
                    } else if (filter == null && ld.hits.isNotEmpty() && ld.pending.isEmpty()) {
                        item(key = "narrow") {
                            Text("Top results from each source. Choose a source above to see more.", style = Type.meta, color = l.p.muted,
                                 modifier = Modifier.padding(horizontal = 16.dp, vertical = 16.dp))
                        }
                    }
                }
            }
        }
    }

    if (managingSources) InkSheet("Book sources", onDismiss = { managingSources = false }) {
        BookSourcesSettings()
    }
    if (addingCatalog) AddCatalogDialog(catalogStore, onDone = { added ->
        addingCatalog = false
        if (added != null) setFilter(BookSource.Opds(added))
    })
    removing?.let { c ->
        InkDialog(
            title = "Remove catalog",
            text = "${c.name} is removed from Folio. Books you downloaded from it stay in your library.",
            confirm = "Remove", onConfirm = { catalogStore.remove(c); removing = null; setFilter(null) },
            onDismiss = { removing = null },
        )
    }
}

/** A stable key for a source (survives recomposition and process restarts). */
private fun sourceKey(s: BookSource): String = when (s) {
    BookSource.StandardEbooks -> "se"
    BookSource.Gutenberg -> "pg"
    is BookSource.Opds -> s.catalog.url
    is BookSource.Wikisource -> "ws"
    BookSource.GutenbergAustralia -> "pga"
    BookSource.FadedPage -> "fp"
    BookSource.InternetArchive -> "ia"
}

/**
 * Which archives Get Books searches. Standard Ebooks and Project Gutenberg are always on; the others
 * follow other countries' copyright rules; they start on, with a note on what that means.
 */
@Composable
fun BookSourcesSettings() {
    val l = look
    val context = LocalContext.current
    val store = remember { SourceStore.get(context) }
    val s by store.settings.collectAsState()
    val languages = remember { uniffi.engine_ffi.wikisourceLanguages() }
    Column(Modifier.padding(horizontal = 16.dp)) {
        CheckRow("Standard Ebooks and Project Gutenberg", "Always on. Public domain in the United States.", value = true, enabled = false) {}
        CheckRow("Wikisource", "Public-domain texts transcribed and proofread by volunteers; downloaded as EPUB. Texts CC BY-SA.",
                 value = s.wikisource) { v -> store.update { it.copy(wikisource = v) } }
        if (s.wikisource) {
            Row(Modifier.padding(bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Picker(languages.firstOrNull { it.code == s.wikisourceLang } ?: languages.first(), languages, { it.name },
                       { lang -> store.update { it.copy(wikisourceLang = lang.code) } }, prefix = "Language")
            }
        }
        Rule(color = l.p.faint)
        Text("These follow other countries' copyright terms. A book free there may still be under copyright where you live; " +
             "check before downloading.", style = Type.meta, color = l.p.muted, modifier = Modifier.padding(top = 12.dp, bottom = 4.dp))
        CheckRow("Project Gutenberg Australia", "Public domain in Australia (authors who died before 1955). Web-page editions. " +
                 "Its full list (under 1 MB) is kept on the phone.", value = s.gutenbergAustralia) { v -> store.update { it.copy(gutenbergAustralia = v) } }
        CheckRow("Faded Page (Canada)", "Public domain in Canada (generally authors who died before 1972). " +
                 "Its full list (about 2 MB) is kept on the phone.", value = s.fadedPage) { v -> store.update { it.copy(fadedPage = v) } }
        CheckRow("Internet Archive", "Scanned books published before 1930 or marked public domain. Scan quality varies.",
                 value = s.internetArchive) { v -> store.update { it.copy(internetArchive = v) } }
        Spacer(Modifier.height(16.dp))
    }
}

@Composable
private fun FoundRow(b: FoundBook, from: String?, login: Login?, onRead: (String) -> Unit) {
    val l = look
    val context = LocalContext.current
    val library = remember { Library.get(context) }
    val books by library.books.collectAsState()
    val downloads by Downloads.state.collectAsState()
    val key = Downloads.key(b)
    val dl = downloads[key]
    val have = dl?.bookId ?: books.firstOrNull { it.source == key }?.id
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(
            Modifier.width(52.dp).aspectRatio(2f / 3f).clip(l.smallShape).background(l.p.well)
                .then(if (l.theme.id == "eink") Modifier.border(1.dp, l.p.ink, RectangleShape) else Modifier),
        ) {
            if (b.cover.startsWith("http")) RemoteImage(b.cover, Modifier.fillMaxSize())
        }
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Text(b.title, style = Type.title, color = l.p.ink, maxLines = 3, overflow = TextOverflow.Ellipsis)
            if (b.author.isNotBlank()) {
                Text(b.author, style = Type.meta, color = l.p.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            val note = dl?.error ?: from
            note?.let {
                Spacer(Modifier.height(3.dp))
                Text(if (dl?.error != null) it else l.caps(it), style = if (dl?.error != null) Type.meta.copy(fontSize = 11.sp) else Type.label.copy(fontSize = 10.sp),
                     maxLines = 2, color = if (dl?.error != null) l.p.accent else l.p.muted)
            }
        }
        Spacer(Modifier.width(10.dp))
        when {
            have != null -> InkButton("Read", { onRead(have) }, filled = true, compact = true)
            dl != null && dl.error == null -> ProgressButton(dl.fraction, { Downloads.cancel(b) })
            dl?.error != null -> InkButton("Retry", { Downloads.clear(b); Downloads.start(context, b, login) }, compact = true)
            else -> InkButton("Get", { Downloads.start(context, b, login) }, compact = true)
        }
    }
}

@Composable
private fun AddCatalogDialog(store: CatalogStore, onDone: (Catalog?) -> Unit) {
    val l = look
    val scope = rememberCoroutineScope()
    var name by remember { mutableStateOf("") }
    var url by remember { mutableStateOf("") }
    var user by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    InkDialog(
        title = "Add a catalog",
        confirm = if (busy) "Checking…" else "Add",
        onConfirm = {
            if (url.isBlank() || busy) return@InkDialog
            busy = true
            scope.launch {
                store.add(name, url, user, password).onSuccess { onDone(it) }.onFailure { error = it.message }
                busy = false
            }
        },
        onDismiss = { if (!busy) onDone(null) },
        body = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("An OPDS catalog, such as Calibre's content server or another library server.", style = Type.meta, color = l.p.muted)
                InkField(url, { url = it; error = null }, "Address, e.g. 192.168.1.10:8080/opds", prompt = "",
                         keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri))
                InkField(name, { name = it }, "Name (optional)", prompt = "")
                InkField(user, { user = it }, "User name (if needed)", prompt = "")
                InkField(password, { password = it }, "Password", prompt = "", visual = PasswordVisualTransformation(),
                         keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password))
                error?.let { Text(it, style = Type.meta, color = l.p.accent) }
            }
        },
    )
}

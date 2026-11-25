package io.folio.reader

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import uniffi.engine_ffi.CatalogPage
import uniffi.engine_ffi.FoundBook
import uniffi.engine_ffi.FoundPage
import uniffi.engine_ffi.bookFileName
import uniffi.engine_ffi.fadedPageListAddress
import uniffi.engine_ffi.gutenbergAustraliaCatalogAddress
import uniffi.engine_ffi.internetArchiveAddress
import uniffi.engine_ffi.parseFadedPageList
import uniffi.engine_ffi.parseGutenbergAustraliaCatalog
import uniffi.engine_ffi.parseInternetArchive
import uniffi.engine_ffi.parseWikisource
import uniffi.engine_ffi.searchCatalog
import uniffi.engine_ffi.wikisourceAddress
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.withPermit
import uniffi.engine_ffi.catalogSearchAddress
import uniffi.engine_ffi.gutenbergAddress
import uniffi.engine_ffi.parseCatalog
import uniffi.engine_ffi.parseGutenberg
import uniffi.engine_ffi.parseStandardEbooks
import uniffi.engine_ffi.standardEbooksAddress
import java.io.File

/** An OPDS catalog the reader added (Calibre's content server, a home library server...). */
data class Catalog(val name: String, val url: String, val user: String = "", val password: String = "") {
    val login: Login? get() = if (user.isEmpty()) null else Login(user, password)
}

/** Where books come from. */
sealed interface BookSource {
    val label: String

    data object StandardEbooks : BookSource {
        override val label = "Standard Ebooks"
    }

    data object Gutenberg : BookSource {
        override val label = "Project Gutenberg"
    }

    data class Opds(val catalog: Catalog) : BookSource {
        override val label get() = catalog.name
    }

    /** Wikisource in one language edition */
    data class Wikisource(val lang: String) : BookSource {
        override val label get() = "Wikisource"
    }

    /** optional archives (public domain under their own country's law) */
    data object GutenbergAustralia : BookSource {
        override val label = "Gutenberg Australia"
    }

    data object FadedPage : BookSource {
        override val label = "Faded Page"
    }

    data object InternetArchive : BookSource {
        override val label = "Internet Archive"
    }
}

/** Which sources are on. Standard Ebooks and Gutenberg always are. */
data class SourceSettings(
    val wikisource: Boolean = true,
    val wikisourceLang: String = "en",
    val gutenbergAustralia: Boolean = true,
    val fadedPage: Boolean = true,
    val internetArchive: Boolean = true,
) {
    /** the enabled sources beyond the two built in, in the order they're listed */
    fun extra(): List<BookSource> = listOfNotNull(
        if (wikisource) BookSource.Wikisource(wikisourceLang) else null,
        if (gutenbergAustralia) BookSource.GutenbergAustralia else null,
        if (fadedPage) BookSource.FadedPage else null,
        if (internetArchive) BookSource.InternetArchive else null,
    )
}

class SourceStore private constructor(context: Context) {
    private val prefs = context.getSharedPreferences("sources", Context.MODE_PRIVATE)
    private val _s = MutableStateFlow(read())
    val settings: StateFlow<SourceSettings> = _s

    private fun read(): SourceSettings {
        // Wikisource in the phone's language when it has an edition in it
        val phone = java.util.Locale.getDefault().language
        val lang = prefs.getString("wsLang", null)
            ?: uniffi.engine_ffi.wikisourceLanguages().firstOrNull { it.code == phone }?.code ?: "en"
        return SourceSettings(
            wikisource = prefs.getBoolean("ws", true), wikisourceLang = lang,
            gutenbergAustralia = prefs.getBoolean("pga", true), fadedPage = prefs.getBoolean("fp", true),
            internetArchive = prefs.getBoolean("ia", true),
        )
    }

    fun update(f: (SourceSettings) -> SourceSettings) {
        val s = f(_s.value)
        _s.value = s
        prefs.edit().putBoolean("ws", s.wikisource).putString("wsLang", s.wikisourceLang).putBoolean("pga", s.gutenbergAustralia)
            .putBoolean("fp", s.fadedPage).putBoolean("ia", s.internetArchive).apply()
    }

    companion object {
        @Volatile private var instance: SourceStore? = null
        fun get(context: Context): SourceStore =
            instance ?: synchronized(this) { instance ?: SourceStore(context.applicationContext).also { instance = it } }
    }
}

/**
 * Whole catalogues kept on the device (Gutenberg Australia, Faded Page), searched locally. Fetched
 * once and refreshed weekly; a fetch carries on in the background even when a search stops waiting.
 */
object CatalogCache {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val loading = HashMap<String, kotlinx.coroutines.Deferred<List<FoundBook>>>()
    private val memory = HashMap<String, List<FoundBook>>()
    private const val MAX_AGE_MS = 7L * 24 * 3600 * 1000
    private lateinit var dir: File

    fun init(context: Context) {
        dir = File(context.filesDir, "catalogs").apply { mkdirs() }
    }

    suspend fun books(source: BookSource): List<FoundBook> {
        val name = when (source) {
            BookSource.GutenbergAustralia -> "gutenberg-australia"
            BookSource.FadedPage -> "faded-page"
            else -> error("not a downloaded catalogue")
        }
        val d = synchronized(this) {
            memory[name]?.let { return it }
            loading[name]?.takeIf { it.isActive } ?: scope.async { load(name, source) }.also { loading[name] = it }
        }
        return d.await()
    }

    private suspend fun load(name: String, source: BookSource): List<FoundBook> {
        val file = File(dir, "$name.json")
        val fresh = file.exists() && System.currentTimeMillis() - file.lastModified() < MAX_AGE_MS
        val books = if (fresh) readFile(file) else try {
            fetch(source).also { writeFile(file, it) }
        } catch (e: Exception) {
            if (file.exists()) readFile(file) else throw e // an old copy beats nothing
        }
        synchronized(this) { memory[name] = books }
        return books
    }

    private suspend fun fetch(source: BookSource): List<FoundBook> = when (source) {
        BookSource.GutenbergAustralia -> parseGutenbergAustraliaCatalog(Http.get(gutenbergAustraliaCatalogAddress()).text)
        BookSource.FadedPage -> coroutineScope {
            val first = parseFadedPageList(Http.get(fadedPageListAddress(0u)).text)
            val gate = kotlinx.coroutines.sync.Semaphore(4)
            val rest = (250 until first.total.toInt() step 250).map { start ->
                async { gate.withPermit { parseFadedPageList(Http.get(fadedPageListAddress(start.toUInt())).text).books } }
            }.awaitAll()
            first.books + rest.flatten()
        }
        else -> emptyList()
    }

    private fun writeFile(f: File, books: List<FoundBook>) {
        val a = JSONArray()
        books.forEach { b ->
            a.put(JSONObject().put("t", b.title).put("a", b.author).put("i", b.info).put("u", JSONArray(b.urls)).put("e", b.ext).put("c", b.cover))
        }
        val tmp = File(f.parentFile, f.name + ".tmp")
        tmp.writeText(a.toString())
        tmp.renameTo(f)
    }

    private fun readFile(f: File): List<FoundBook> {
        val a = JSONArray(f.readText())
        return (0 until a.length()).map { i ->
            val o = a.getJSONObject(i)
            val u = o.getJSONArray("u")
            FoundBook(o.getString("t"), o.optString("a"), o.optString("i"), (0 until u.length()).map { u.getString(it) },
                      o.optString("e", "epub"), o.optString("c"))
        }
    }
}

/** One page from a source: books, sub-catalogs (OPDS), and where to go for more. */
data class SourcePage(
    val title: String,
    val books: List<FoundBook>,
    val folders: List<Pair<String, String>>, // (title, address)
    val next: String?,
    /** OPDS: how to search this catalog */
    val search: String?,
    /** a line to show instead of results, e.g. "Search Wikisource…" */
    val hint: String? = null,
)

object Sources {
    /** The first page: what's new or popular, or the results of a search. */
    suspend fun open(source: BookSource, query: String): SourcePage = when (source) {
        is BookSource.Wikisource ->
            if (query.isBlank()) hint("Search Wikisource: works in the public domain, proofread by volunteers.")
            else books(parseWikisource(source.lang, Http.get(wikisourceAddress(source.lang, query)).text))
        BookSource.GutenbergAustralia, BookSource.FadedPage -> {
            val all = CatalogCache.books(source)
            books(searchCatalog(all, query, 80u))
        }
        BookSource.InternetArchive ->
            if (query.isBlank()) hint("Search the Internet Archive's public-domain books (published before 1930 or marked public domain).")
            else books(parseInternetArchive(Http.get(internetArchiveAddress(query)).text))
        BookSource.StandardEbooks -> fromFound(parseStandardEbooks(Http.get(standardEbooksAddress(query)).text))
        BookSource.Gutenberg -> fromFound(parseGutenberg(Http.get(gutenbergAddress(query)).text))
        is BookSource.Opds -> {
            val c = source.catalog
            if (query.isBlank()) {
                page(c, c.url)
            } else {
                // the catalog's front page says how to search it
                val front = parseCatalog(Http.get(c.url, login = c.login).text, c.url, c.name)
                val link = front.search ?: error("This catalog doesn't offer search.")
                val desc = if (link.contains("{searchTerms}")) null else Http.get(link, login = c.login).text
                val address = catalogSearchAddress(link, desc, query) ?: error("This catalog doesn't offer search.")
                page(c, address)
            }
        }
    }

    /** A further page: the next page of results, or a sub-catalog. */
    suspend fun more(source: BookSource, address: String): SourcePage = when (source) {
        is BookSource.Wikisource, BookSource.GutenbergAustralia, BookSource.FadedPage, BookSource.InternetArchive -> books(emptyList())
        BookSource.Gutenberg -> fromFound(parseGutenberg(Http.get(address).text))
        BookSource.StandardEbooks -> fromFound(parseStandardEbooks(Http.get(address).text))
        is BookSource.Opds -> page(source.catalog, address)
    }

    private suspend fun page(c: Catalog, address: String): SourcePage {
        val f = Http.get(address, login = c.login)
        val p: CatalogPage = parseCatalog(f.text, f.url, c.name)
        return SourcePage(
            title = p.title.ifBlank { c.name },
            books = p.entries.mapNotNull { it.book },
            folders = p.entries.filter { it.book == null && it.nav != null }.map { it.title to it.nav!! },
            next = p.next, search = p.search,
        )
    }

    private fun fromFound(p: FoundPage) = SourcePage("", p.books, emptyList(), p.next, null)
    private fun books(b: List<FoundBook>) = SourcePage("", b, emptyList(), null, null)
    private fun hint(text: String) = SourcePage("", emptyList(), emptyList(), null, null, hint = text)
}

class CatalogStore private constructor(context: Context) {
    private val prefs = context.getSharedPreferences("catalogs", Context.MODE_PRIVATE)
    private val _list = MutableStateFlow(read())
    val catalogs: StateFlow<List<Catalog>> = _list

    private fun read(): List<Catalog> = try {
        val a = JSONArray(prefs.getString("list", "[]"))
        (0 until a.length()).map {
            val o = a.getJSONObject(it)
            Catalog(o.getString("name"), o.getString("url"), o.optString("user"), o.optString("password"))
        }
    } catch (e: Exception) {
        emptyList()
    }

    private fun write(list: List<Catalog>) {
        _list.value = list
        val a = JSONArray()
        list.forEach { a.put(JSONObject().put("name", it.name).put("url", it.url).put("user", it.user).put("password", it.password)) }
        prefs.edit().putString("list", a.toString()).apply()
    }

    /** Check the address answers as an OPDS catalog, then keep it. */
    suspend fun add(name: String, address: String, user: String, password: String): Result<Catalog> = runCatching {
        val url = address.trim().let { if (it.contains("://")) it else "https://$it" }
        val c = Catalog(name.trim(), url, user.trim(), password)
        val f = Http.get(url, login = c.login)
        val page = parseCatalog(f.text, f.url, c.name)
        if (page.entries.isEmpty() && page.title.isBlank()) error("This address didn't answer as an OPDS catalog.")
        val named = c.copy(name = c.name.ifBlank { page.title.ifBlank { url.substringAfter("://").substringBefore('/') } })
        write(_list.value.filter { it.url != named.url } + named)
        named
    }.recoverCatching { throw Exception(it.friendly()) }

    fun remove(c: Catalog) = write(_list.value - c)

    companion object {
        @Volatile private var instance: CatalogStore? = null
        fun get(context: Context): CatalogStore =
            instance ?: synchronized(this) { instance ?: CatalogStore(context.applicationContext).also { instance = it } }
    }
}

/** The state of one download. */
data class Download(
    val title: String,
    /** 0..1, or null while the size is unknown */
    val fraction: Float? = null,
    val bytes: Long = 0,
    val error: String? = null,
    /** the library book, once added */
    val bookId: String? = null,
)

/**
 * Book downloads. They run in the app's own scope, so they finish even when the screen that
 * started them is gone, and each finished file goes straight into the library.
 */
object Downloads {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val jobs = mutableMapOf<String, Job>()
    private val _state = MutableStateFlow<Map<String, Download>>(emptyMap())
    val state: StateFlow<Map<String, Download>> = _state

    /** The key for a book: its first download address. */
    fun key(b: FoundBook) = b.urls.firstOrNull() ?: b.title

    fun start(context: Context, b: FoundBook, login: Login?) {
        val key = key(b)
        if (jobs[key]?.isActive == true) return
        val library = Library.get(context)
        _state.update { it + (key to Download(b.title)) }
        jobs[key] = scope.launch {
            val dir = File(context.cacheDir, "downloads").apply { mkdirs() }
            val name = bookFileName(b.title, b.author, b.ext)
            var lastError: Throwable? = null
            for (url in b.urls) {
                val dest = File(dir, "${System.nanoTime()}.${b.ext}")
                try {
                    Http.download(url, dest, login) { done, total ->
                        _state.update { m ->
                            m + (key to Download(b.title, if (total > 0) (done.toFloat() / total).coerceIn(0f, 1f) else null, done))
                        }
                    }
                    val book = library.importFile(dest, name, source = key).getOrThrow()
                    _state.update { it + (key to Download(b.title, 1f, bookId = book.id)) }
                    return@launch
                } catch (e: kotlinx.coroutines.CancellationException) {
                    dest.delete()
                    _state.update { it - key }
                    throw e
                } catch (e: Exception) {
                    dest.delete()
                    lastError = e
                }
            }
            _state.update { it + (key to Download(b.title, error = lastError?.friendly() ?: "The download failed.")) }
        }
    }

    fun cancel(b: FoundBook) {
        jobs.remove(key(b))?.cancel()
        _state.update { it - key(b) }
    }

    /** Forget a failed download (so it can be tried again). */
    fun clear(b: FoundBook) {
        _state.update { it - key(b) }
    }
}

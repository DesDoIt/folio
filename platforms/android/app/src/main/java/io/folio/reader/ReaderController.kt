package io.folio.reader

import android.graphics.Bitmap
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.withContext
import uniffi.engine_ffi.FontFiles
import uniffi.engine_ffi.PageStyle
import uniffi.engine_ffi.Position
import uniffi.engine_ffi.ReadingSession
import uniffi.engine_ffi.SearchResult
import uniffi.engine_ffi.TapResult
import uniffi.engine_ffi.TocItem
import uniffi.engine_ffi.WordHit
import java.nio.ByteBuffer

/** What the reader screen shows; replaced as a whole so Compose sees consistent frames. */
data class Pages(
    val prev: ImageBitmap? = null,
    val current: ImageBitmap? = null,
    val next: ImageBitmap? = null,
    val chapterTitle: String = "",
    val page: Int = 0,
    val pageCount: Int = 0,
    val progress: Float = 0f,
    val bookmarked: Boolean = false,
    /** the document has more than one chapter (an article has one) */
    val chaptered: Boolean = true,
)

/** The page geometry and look requested by the screen. */
data class PageSpec(
    val width: Int,
    val height: Int,
    val style: PageStyle,
    val theme: PageTheme,
    val typeface: Typeface,
)

/** Where a reading source keeps its place and bookmarks. */
interface ReadingState {
    fun position(): Position?
    fun save(position: Position, progress: Float)
    fun bookmarks(): List<Bookmark>
    fun saveBookmarks(list: List<Bookmark>)
    val supportsBookmarks: Boolean get() = true
}

/** A book in the library. */
class BookState(private val library: Library, private val id: String) : ReadingState {
    override fun position() = library.position(id)
    override fun save(position: Position, progress: Float) {
        library.savePosition(id, position)
        library.markOpened(id, progress)
    }
    override fun bookmarks() = library.bookmarks(id)
    override fun saveBookmarks(list: List<Bookmark>) = library.saveBookmarks(id, list)
}

/** What is being read: its title, how to open it, and where its place is kept. */
class ReadingSource(
    val title: String,
    val author: String,
    val state: ReadingState,
    /** opens the session; may fetch over the network first */
    val open: suspend (FontFiles) -> ReadingSession,
) {
    companion object {
        fun book(library: Library, book: Book) = ReadingSource(book.title, book.author, BookState(library, book.id)) { fonts ->
            ReadingSession.open(book.file, fonts)
        }
    }
}

/**
 * One open book or article. Every engine call runs on a single background thread, in order; the
 * screen observes [pages]. The neighbouring pages are kept rendered so turns and swipes are instant.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ReaderController(val source: ReadingSource) {
    private val store = source.state
    private val engine = Dispatchers.Default.limitedParallelism(1)
    private var session: ReadingSession? = null
    private var spec: PageSpec? = null
    private var bookmarks: List<Bookmark> = store.bookmarks()
    private var chaptered = true

    var pages by mutableStateOf(Pages())
        private set
    var error by mutableStateOf<String?>(null)
        private set

    private fun fonts(t: Typeface): FontFiles = FontBank.files(t)

    /** Open the book (if needed) and lay it out for this page spec. */
    suspend fun configure(s: PageSpec) = withContext(engine) {
        val old = spec
        if (old == s && session != null) return@withContext
        val sess = session ?: try {
            source.open(fonts(s.typeface)).also { ses ->
                session = ses
                chaptered = ses.toc().size > 1
                store.position()?.let { ses.goto(it) }
            }
        } catch (e: Exception) {
            error = e.message ?: "This book couldn't be opened."
            return@withContext
        }
        if (old != null && old.typeface != s.typeface) sess.setFonts(fonts(s.typeface))
        sess.setViewport(s.width, s.height)
        sess.setStyle(s.style)
        spec = s
        renderAll(sess, s)
    }

    private fun bitmap(sess: ReadingSession, delta: Int, s: PageSpec): ImageBitmap? {
        val px = sess.render(delta, uniffi.engine_ffi.PageTheme(s.theme.paperRgb, s.theme.inkRgb, true)) ?: return null
        val bmp = Bitmap.createBitmap(s.width, s.height, Bitmap.Config.ARGB_8888)
        bmp.copyPixelsFromBuffer(ByteBuffer.wrap(px))
        return bmp.asImageBitmap()
    }

    private fun meta(sess: ReadingSession, base: Pages): Pages {
        val pc = sess.pageInChapter()
        val here = bookmarks.any { sess.onCurrentPage(it.position) }
        return base.copy(chapterTitle = sess.chapterTitle(), page = pc[0].toInt(), pageCount = pc[1].toInt(),
                         progress = sess.progress(), bookmarked = here, chaptered = chaptered)
    }

    private fun renderAll(sess: ReadingSession, s: PageSpec) {
        val cur = bitmap(sess, 0, s)
        pages = meta(sess, Pages(current = cur, prev = pages.prev, next = pages.next))
        pages = pages.copy(prev = bitmap(sess, -1, s), next = bitmap(sess, 1, s))
        persist(sess)
    }

    /**
     * Turn one page forward (+1) or back (-1). False at either end of the book. [onShifted] runs
     * in the same snapshot that swaps the pages, so a slide animation can reset without a flicker.
     */
    suspend fun turn(delta: Int, onShifted: () -> Unit = {}): Boolean = withContext(engine) {
        val sess = session ?: return@withContext false
        val s = spec ?: return@withContext false
        if (!sess.turn(delta)) {
            Snapshot.withMutableSnapshot { onShifted() }
            return@withContext false
        }
        val p = pages
        // show the already-rendered neighbour at once, then render the new neighbour
        val shifted = meta(sess, if (delta > 0) p.copy(prev = p.current, current = p.next, next = null)
                                 else p.copy(next = p.current, current = p.prev, prev = null))
        Snapshot.withMutableSnapshot {
            pages = shifted
            onShifted()
        }
        if (delta > 0) pages = pages.copy(next = bitmap(sess, 1, s)) else pages = pages.copy(prev = bitmap(sess, -1, s))
        if (pages.current == null) renderAll(sess, s)
        persist(sess)
        true
    }

    suspend fun goto(p: Position) = withContext(engine) {
        val sess = session ?: return@withContext
        sess.goto(p)
        spec?.let { renderAll(sess, it) }
    }

    suspend fun gotoProgress(f: Float) = withContext(engine) {
        val sess = session ?: return@withContext
        sess.gotoProgress(f)
        spec?.let { renderAll(sess, it) }
    }

    suspend fun position(): Position? = withContext(engine) { session?.position() }

    suspend fun tap(x: Int, y: Int): TapResult = withContext(engine) { session?.tap(x, y) ?: TapResult.Nothing }

    suspend fun wordAt(x: Int, y: Int): WordHit? = withContext(engine) { session?.wordAt(x, y) }

    suspend fun toc(): List<TocItem> = withContext(engine) { session?.toc() ?: emptyList() }

    suspend fun search(q: String): List<SearchResult> = withContext(engine) { session?.search(q, 500u) ?: emptyList() }

    fun bookmarks(): List<Bookmark> = bookmarks

    /** Add a bookmark on the current page, or remove the ones on it. */
    suspend fun toggleBookmark() = withContext(engine) {
        val sess = session ?: return@withContext
        val here = bookmarks.filter { sess.onCurrentPage(it.position) }
        bookmarks = if (here.isNotEmpty()) bookmarks - here.toSet()
                    else bookmarks + Bookmark(sess.position(), sess.pageSnippet(12u), System.currentTimeMillis())
        store.saveBookmarks(bookmarks)
        pages = pages.copy(bookmarked = here.isEmpty())
    }

    suspend fun removeBookmark(b: Bookmark) = withContext(engine) {
        bookmarks = bookmarks - b
        store.saveBookmarks(bookmarks)
        session?.let { sess -> pages = pages.copy(bookmarked = bookmarks.any { sess.onCurrentPage(it.position) }) }
    }

    private fun persist(sess: ReadingSession) {
        store.save(sess.position(), sess.progress())
    }

    fun close() {
        session?.destroy()
        session = null
    }
}

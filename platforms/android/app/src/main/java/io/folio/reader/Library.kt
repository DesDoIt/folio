package io.folio.reader

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import uniffi.engine_ffi.Position
import uniffi.engine_ffi.bookMeta
import java.io.File
import java.security.MessageDigest

/** File types the engine reads. */
val BOOK_EXTENSIONS = listOf("epub", "mobi", "azw3", "azw", "prc", "fb2", "fb2.zip", "rtf", "docx", "txt", "html", "htm")

fun bookExtension(name: String): String? {
    val lower = name.lowercase()
    return BOOK_EXTENSIONS.sortedByDescending { it.length }.firstOrNull { lower.endsWith(".$it") }
}

data class Book(
    val id: String,
    val file: String,
    val title: String,
    val author: String,
    val added: Long,
    val opened: Long,
    val progress: Float,
    val hasCover: Boolean,
    /** where it was downloaded from, if Folio downloaded it */
    val source: String = "",
) {
    fun toJson(): JSONObject = JSONObject()
        .put("id", id).put("file", file).put("title", title).put("author", author)
        .put("added", added).put("opened", opened).put("progress", progress.toDouble()).put("hasCover", hasCover)
        .put("source", source)

    companion object {
        fun fromJson(o: JSONObject) = Book(
            id = o.getString("id"), file = o.getString("file"), title = o.optString("title"),
            author = o.optString("author"), added = o.optLong("added"), opened = o.optLong("opened"),
            progress = o.optDouble("progress", 0.0).toFloat(), hasCover = o.optBoolean("hasCover"),
            source = o.optString("source"),
        )
    }
}

data class Bookmark(val position: Position, val label: String, val created: Long)

/**
 * The books the user has added: copies kept in app storage (so they stay readable whatever happens
 * to the original), with title, author, cover and reading progress in a small JSON index.
 */
class Library private constructor(private val context: Context) {
    companion object {
        @Volatile private var instance: Library? = null
        fun get(context: Context): Library =
            instance ?: synchronized(this) { instance ?: Library(context.applicationContext).also { instance = it } }
    }

    private val root = context.filesDir
    private val booksDir = File(root, "books").apply { mkdirs() }
    private val coversDir = File(root, "covers").apply { mkdirs() }
    private val stateDir = File(root, "state").apply { mkdirs() }
    private val index = File(root, "library.json")

    private val _books = MutableStateFlow(load())
    val books: StateFlow<List<Book>> = _books

    private fun load(): List<Book> = try {
        val a = JSONArray(index.readText())
        (0 until a.length()).map { Book.fromJson(a.getJSONObject(it)) }.filter { File(it.file).exists() }
    } catch (e: Exception) {
        emptyList()
    }

    @Synchronized
    private fun save(list: List<Book>) {
        _books.value = list
        val a = JSONArray()
        list.forEach { a.put(it.toJson()) }
        val tmp = File(root, "library.json.tmp")
        tmp.writeText(a.toString())
        tmp.renameTo(index)
    }

    fun get(id: String): Book? = _books.value.firstOrNull { it.id == id }

    fun coverFile(id: String) = File(coversDir, id)

    /** Copy a book in from a content or file URI; returns it, or an error message. */
    suspend fun import(uri: Uri): Result<Book> = withContext(Dispatchers.IO) {
        runCatching {
            val name = displayName(uri) ?: uri.lastPathSegment ?: "book"
            val ext = bookExtension(name) ?: sniffExtension(uri)
                ?: error("Folio can't open “$name”. It reads EPUB, MOBI, AZW3, FB2, RTF, DOCX and TXT.")
            val tmp = File(context.cacheDir, "import-${System.nanoTime()}.$ext")
            context.contentResolver.openInputStream(uri).use { input ->
                requireNotNull(input) { "Couldn't read the file." }
                tmp.outputStream().use { input.copyTo(it) }
            }
            add(tmp, name, ext, source = "")
        }
    }

    /** Add a downloaded file (it is moved into the library); [source] is where it came from. */
    suspend fun importFile(file: File, name: String, source: String): Result<Book> = withContext(Dispatchers.IO) {
        runCatching {
            val ext = bookExtension(name) ?: "epub"
            add(file, name, ext, source)
        }
    }

    /** The book downloaded from this address, if it is in the library. */
    fun fromSource(url: String): Book? = _books.value.firstOrNull { it.source.isNotEmpty() && it.source == url }

    private fun add(tmp: File, name: String, ext: String, source: String): Book {
        val id = sha1(tmp).take(16)
        get(id)?.let { tmp.delete(); return it } // already in the library
        val dest = File(booksDir, "$id.$ext")
        tmp.copyTo(dest, overwrite = true)
        tmp.delete()
        val meta = try {
            bookMeta(dest.path)
        } catch (e: Exception) {
            dest.delete()
            error("“$name” couldn't be opened: ${e.message}")
        }
        meta.cover?.let { coverFile(id).writeBytes(it) }
        val now = System.currentTimeMillis()
        val book = Book(
            id = id, file = dest.path,
            title = meta.title.ifBlank { name.substringBeforeLast('.') },
            author = meta.author, added = now, opened = 0, progress = 0f, hasCover = meta.cover != null, source = source,
        )
        save(listOf(book) + _books.value)
        return book
    }

    fun delete(id: String) {
        get(id)?.let { File(it.file).delete() }
        coverFile(id).delete()
        File(stateDir, "$id.json").delete()
        save(_books.value.filter { it.id != id })
    }

    fun markOpened(id: String, progress: Float) {
        save(_books.value.map { if (it.id == id) it.copy(opened = System.currentTimeMillis(), progress = progress) else it })
    }

    // --- per-book reading state: position and bookmarks ---

    private fun stateFile(id: String) = File(stateDir, "$id.json")

    private fun readState(id: String): JSONObject =
        try { JSONObject(stateFile(id).readText()) } catch (e: Exception) { JSONObject() }

    @Synchronized
    private fun writeState(id: String, o: JSONObject) {
        val tmp = File(stateDir, "$id.json.tmp")
        tmp.writeText(o.toString())
        tmp.renameTo(stateFile(id))
    }

    fun position(id: String): Position? {
        val o = readState(id).optJSONObject("position") ?: return null
        return Position(o.optLong("chapter").toULong(), o.optLong("offset").toULong())
    }

    fun savePosition(id: String, p: Position) {
        val o = readState(id)
        o.put("position", JSONObject().put("chapter", p.chapter.toLong()).put("offset", p.offset.toLong()))
        writeState(id, o)
    }

    fun bookmarks(id: String): List<Bookmark> {
        val a = readState(id).optJSONArray("bookmarks") ?: return emptyList()
        return (0 until a.length()).map {
            val b = a.getJSONObject(it)
            Bookmark(Position(b.getLong("chapter").toULong(), b.getLong("offset").toULong()), b.optString("label"), b.optLong("created"))
        }.sortedWith(compareBy({ it.position.chapter }, { it.position.offset }))
    }

    fun saveBookmarks(id: String, list: List<Bookmark>) {
        val o = readState(id)
        val a = JSONArray()
        list.forEach {
            a.put(JSONObject().put("chapter", it.position.chapter.toLong()).put("offset", it.position.offset.toLong())
                .put("label", it.label).put("created", it.created))
        }
        o.put("bookmarks", a)
        writeState(id, o)
    }

    private fun displayName(uri: Uri): String? = try {
        context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0) else null
        }
    } catch (e: Exception) {
        null
    }

    /** For files without a usable name: recognise EPUB/DOCX (zip) and MOBI by their bytes. */
    private fun sniffExtension(uri: Uri): String? = try {
        context.contentResolver.openInputStream(uri)?.use { s ->
            val head = ByteArray(100)
            val n = s.read(head)
            when {
                n >= 4 && head[0] == 'P'.code.toByte() && head[1] == 'K'.code.toByte() ->
                    if (String(head, 30, minOf(n, 100) - 30, Charsets.ISO_8859_1).contains("mimetypeapplication/epub")) "epub" else null
                n >= 68 && String(head, 60, 8, Charsets.ISO_8859_1) == "BOOKMOBI" -> "mobi"
                else -> null
            }
        }
    } catch (e: Exception) {
        null
    }

    private fun sha1(f: File): String {
        val md = MessageDigest.getInstance("SHA-1")
        f.inputStream().use { s ->
            val buf = ByteArray(1 shl 16)
            while (true) {
                val n = s.read(buf)
                if (n <= 0) break
                md.update(buf, 0, n)
            }
        }
        return md.digest().joinToString("") { "%02x".format(it) }
    }
}

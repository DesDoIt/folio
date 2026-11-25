package io.folio.reader

import android.graphics.BitmapFactory
import android.net.Uri
import android.util.LruCache
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
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
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlin.math.abs

enum class SortBy(val label: String) { Recent("Recent"), Title("Title"), Author("Author") }

@Composable
fun LibraryScreen(
    library: Library,
    onOpen: (Book) -> Unit,
    onImport: (List<Uri>) -> Unit,
    onFindBooks: () -> Unit,
    onSettings: () -> Unit,
    bottomBar: @Composable () -> Unit = {},
) {
    val l = look
    val books by library.books.collectAsState()
    var sort by rememberSaveable { mutableStateOf(SortBy.Recent) }
    var deleting by remember { mutableStateOf<Book?>(null) }
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isNotEmpty()) onImport(uris)
    }
    val addBooks = { pick.launch(arrayOf("*/*")) }
    val sorted = when (sort) {
        SortBy.Recent -> books.sortedByDescending { maxOf(it.opened, it.added) }
        SortBy.Title -> books.sortedBy { it.title.lowercase().removePrefix("the ").removePrefix("a ") }
        SortBy.Author -> books.sortedBy { it.author.lowercase() }
    }
    val current = books.filter { it.opened > 0 && it.progress < 0.995f }.maxByOrNull { it.opened }
    val list = l.theme.shelf == ShelfStyle.List

    Scaffold(
        containerColor = l.p.paper,
        topBar = {
            Header(listOf("Library")) {
                HeaderIcon(Icons.Outlined.Add, "Add books from your phone", addBooks)
                HeaderIcon(Icons.Outlined.Settings, "Settings", onSettings)
            }
        },
        bottomBar = bottomBar,
    ) { pad ->
        if (books.isEmpty()) {
            EmptyLibrary(Modifier.padding(pad), addBooks, onFindBooks)
            return@Scaffold
        }
        val side = if (list) 0.dp else 16.dp
        LazyVerticalGrid(
            columns = if (list) GridCells.Fixed(1) else GridCells.Adaptive(104.dp),
            contentPadding = PaddingValues(top = pad.calculateTopPadding(), bottom = pad.calculateBottomPadding() + 24.dp,
                                           start = side, end = side),
            horizontalArrangement = Arrangement.spacedBy(side),
            verticalArrangement = Arrangement.spacedBy(if (list) 0.dp else 18.dp),
            modifier = Modifier.fillMaxSize(),
        ) {
            if (current != null) {
                item(span = { GridItemSpan(maxLineSpan) }, key = "now") {
                    NowReading(library, current, Modifier.padding(start = if (list) 16.dp else 0.dp, end = if (list) 16.dp else 0.dp, top = 16.dp)) {
                        onOpen(current)
                    }
                }
            }
            item(span = { GridItemSpan(maxLineSpan) }, key = "shelf") {
                Row(Modifier.fillMaxWidth().padding(start = if (list) 16.dp else 0.dp, end = if (list) 16.dp else 0.dp, top = 20.dp, bottom = 4.dp),
                    verticalAlignment = Alignment.CenterVertically) {
                    Text(l.caps("${books.size} ${if (books.size == 1) "book" else "books"}"), style = Type.label, color = l.p.muted,
                         modifier = Modifier.weight(1f))
                    Picker(sort, SortBy.entries, { it.label }, { sort = it }, prefix = "Sort")
                }
            }
            itemsIndexed(sorted, key = { _, b -> b.id }) { i, b ->
                if (list) {
                    BookRow(library, b, i + 1, onClick = { onOpen(b) }, onLongClick = { deleting = b })
                } else {
                    BookTile(library, b, onClick = { onOpen(b) }, onLongClick = { deleting = b })
                }
            }
        }
    }

    deleting?.let { b ->
        InkDialog(
            title = "Remove book",
            text = "“${b.title}”, your place in it and its bookmarks will be removed from Folio.",
            confirm = "Remove", onConfirm = { library.delete(b.id); deleting = null },
            onDismiss = { deleting = null },
        )
    }
}

@Composable
private fun EmptyLibrary(modifier: Modifier, onAdd: () -> Unit, onFind: () -> Unit) {
    val l = look
    val start = l.theme.id == "eink"
    Column(
        modifier.fillMaxSize().padding(28.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = if (start) Alignment.Start else Alignment.CenterHorizontally,
    ) {
        Text(l.caps("Your shelf is empty"), style = Type.display, color = l.p.ink, textAlign = if (start) TextAlign.Start else TextAlign.Center)
        Spacer(Modifier.height(12.dp))
        Text(
            "Get free books from copyright-free libraries, or add EPUB, MOBI, AZW3, FB2, DOCX or TXT books from your phone.",
            style = Type.body.copy(fontSize = 15.sp, lineHeight = 22.sp), color = l.p.muted,
            textAlign = if (start) TextAlign.Start else TextAlign.Center,
        )
        Spacer(Modifier.height(24.dp))
        InkButton("Get free books", onFind, filled = true)
        Spacer(Modifier.height(10.dp))
        InkButton("Add from your phone", onAdd)
    }
}

@OptIn(ExperimentalFoundationApi::class)
private fun Modifier.press(onClick: () -> Unit, onLongClick: (() -> Unit)? = null): Modifier =
    this.combinedClickable(onClick = onClick, onLongClick = onLongClick)

@Composable
private fun NowReading(library: Library, book: Book, modifier: Modifier, onClick: () -> Unit) {
    val l = look
    Panel(modifier.press(onClick)) {
        Row(Modifier.padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
            Cover(library, book, Modifier.width(62.dp))
            Spacer(Modifier.width(16.dp))
            Column(Modifier.weight(1f)) {
                Text(l.caps("Continue reading"), style = Type.label, color = l.p.accent)
                Spacer(Modifier.height(4.dp))
                Text(book.title, style = Type.title.copy(fontSize = 20.sp, lineHeight = 25.sp), color = l.p.ink,
                     maxLines = 2, overflow = TextOverflow.Ellipsis)
                if (book.author.isNotBlank()) {
                    Spacer(Modifier.height(2.dp))
                    Text(book.author, style = Type.meta, color = l.p.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                Spacer(Modifier.height(12.dp))
                ProgressMeter(book.progress, Modifier.fillMaxWidth())
            }
        }
    }
}

/** E-ink: a numbered, ruled row. */
@Composable
private fun BookRow(library: Library, book: Book, n: Int, onClick: () -> Unit, onLongClick: () -> Unit) {
    val l = look
    Column(Modifier.press(onClick, onLongClick)) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Cover(library, book, Modifier.width(40.dp))
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(book.title, style = Type.title, color = l.p.ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
                if (book.author.isNotBlank()) {
                    Text(book.author, style = Type.meta, color = l.p.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            Spacer(Modifier.width(10.dp))
            when {
                book.opened == 0L -> Tag("New")
                book.progress >= 0.995f -> Tag("Done", inverted = true)
                else -> Text("%03d%%".format((book.progress * 100).toInt()), style = Type.meta.copy(fontWeight = FontWeight.SemiBold), color = l.p.ink)
            }
        }
        Rule(Modifier.padding(start = 70.dp, end = 16.dp), color = l.p.faint)
    }
}

/** Editorial and Warm: a cover on the shelf. */
@Composable
private fun BookTile(library: Library, book: Book, onClick: () -> Unit, onLongClick: () -> Unit) {
    val l = look
    Column(Modifier.press(onClick, onLongClick)) {
        Cover(library, book, Modifier.fillMaxWidth())
        Spacer(Modifier.height(8.dp))
        Text(book.title, style = Type.title.copy(fontSize = 15.sp, lineHeight = 19.sp), color = l.p.ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
        if (book.author.isNotBlank()) {
            Text(book.author, style = Type.meta, color = l.p.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        Spacer(Modifier.height(6.dp))
        when {
            book.opened == 0L -> Tag("New")
            book.progress >= 0.995f -> Tag("Finished", inverted = true)
            else -> ProgressMeter(book.progress, Modifier.fillMaxWidth(), showPercent = false)
        }
    }
}

private val coverCache = LruCache<String, ImageBitmap>(64)

@Composable
fun Cover(library: Library, book: Book, modifier: Modifier) {
    val l = look
    val bitmap by produceState(coverCache.get(book.id), book.id, book.hasCover) {
        if (value == null && book.hasCover) {
            value = withContext(Dispatchers.IO) {
                val f = library.coverFile(book.id)
                val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeFile(f.path, bounds)
                var sample = 1
                while (bounds.outWidth / (sample * 2) >= 360) sample *= 2
                BitmapFactory.decodeFile(f.path, BitmapFactory.Options().apply { inSampleSize = sample })
                    ?.asImageBitmap()?.also { coverCache.put(book.id, it) }
            }
        }
    }
    val shape = if (l.theme.corner > 0.dp) l.smallShape else RectangleShape
    val frame = when (l.theme.id) {
        "eink" -> Modifier.border(1.dp, l.p.ink, RectangleShape)
        "editorial" -> Modifier.shadow(2.dp, shape)
        else -> Modifier.shadow(6.dp, shape)
    }
    Box(modifier.aspectRatio(2f / 3f).then(frame).clip(shape)) {
        val b = bitmap
        if (b != null) {
            Image(b, contentDescription = null, contentScale = ContentScale.Crop, colorFilter = l.imageFilter, modifier = Modifier.fillMaxSize())
        } else {
            GeneratedCover(book)
        }
    }
}

/** A typographic cover for books without one, in the theme's colours. */
@Composable
private fun GeneratedCover(book: Book) {
    val l = look
    val warm = listOf(0xFF7A3E2B, 0xFF2F4F4F, 0xFF3E4C7A, 0xFF5B3A6B, 0xFF4A5D3A, 0xFF7A5A2B, 0xFF2B4A5E)
    val (bg, fg) = when (l.theme.id) {
        "eink" -> l.p.ink to l.p.paper
        "editorial" -> listOf(l.p.accent, Color(0xFF1F3A34), Color(0xFF2A2A44), Color(0xFF4A3B23))[abs(book.title.hashCode()) % 4] to Color(0xFFF8F4EC)
        else -> Color(warm[abs(book.title.hashCode()) % warm.size]) to Color.White
    }
    BoxWithConstraints(Modifier.fillMaxSize().background(bg)) {
        val w = maxWidth.value
        val title = (w * 0.13f).coerceIn(6f, 17f).sp
        val small = (w * 0.085f).coerceIn(5f, 12f).sp
        val pad = (w * 0.08f).coerceIn(3f, 12f).dp
        Column(Modifier.fillMaxSize().padding(pad), verticalArrangement = Arrangement.SpaceBetween) {
            Rule(color = fg.copy(alpha = 0.6f))
            Text(if (l.theme.id == "eink") book.title.uppercase() else book.title, color = fg,
                 style = Type.title.copy(fontFamily = if (l.theme.id == "eink") Mono else l.theme.content, fontSize = title, lineHeight = title * 1.2f),
                 maxLines = 5, overflow = TextOverflow.Ellipsis)
            Column {
                if (w >= 72 && book.author.isNotBlank()) {
                    Text(book.author, color = fg.copy(alpha = 0.85f), fontSize = small, maxLines = 2, overflow = TextOverflow.Ellipsis,
                         fontFamily = l.theme.ui)
                    Spacer(Modifier.height(4.dp))
                }
                Rule(color = fg.copy(alpha = 0.6f))
            }
        }
    }
}

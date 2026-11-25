package io.folio.reader

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.widget.Toast
import androidx.compose.foundation.border
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.text.TextStyle
import androidx.compose.material3.Slider
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.navigationBarsIgnoringVisibility
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.statusBarsIgnoringVisibility
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.List
import androidx.compose.material.icons.filled.Bookmark
import androidx.compose.material.icons.outlined.BookmarkBorder
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.TextFields
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.PrimaryTabRow
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import uniffi.engine_ffi.PageStyle
import uniffi.engine_ffi.SearchResult
import uniffi.engine_ffi.TapResult
import uniffi.engine_ffi.TocItem
import uniffi.engine_ffi.WordHit
import kotlin.math.abs
import kotlin.math.roundToInt

private enum class Sheet { None, Contents, Settings, Search }

@OptIn(ExperimentalLayoutApi::class, ExperimentalMaterial3Api::class)
@Composable
fun ReaderScreen(
    settings: SettingsStore,
    source: ReadingSource,
    key: String,
    volumeTurns: SharedFlow<Int>,
    onClose: () -> Unit,
) {
    val controller = remember(key) { ReaderController(source) }
    DisposableEffect(controller) { onDispose { controller.close() } }
    val prefs by settings.settings.collectAsState()
    val systemDark = androidx.compose.foundation.isSystemInDarkTheme()
    val theme = prefs.effectiveTheme(systemDark)
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val density = LocalDensity.current

    var controls by remember { mutableStateOf(false) }
    var sheet by remember { mutableStateOf(Sheet.None) }
    var note by remember { mutableStateOf<TapResult.Note?>(null) }
    var external by remember { mutableStateOf<String?>(null) }
    var word by remember { mutableStateOf<WordHit?>(null) }
    var searchQuery by remember { mutableStateOf("") }
    var offset by remember { mutableFloatStateOf(0f) }
    val turnLock = remember { Mutex() }
    val pages = controller.pages

    SystemBars(visible = controls || sheet != Sheet.None, lightBars = !theme.dark, keepOn = prefs.keepScreenOn)

    // insets that don't change when the bars hide, so pages keep their layout
    val topInset = WindowInsets.statusBarsIgnoringVisibility.getTop(density)
        .coerceAtLeast(WindowInsets.displayCutout.getTop(density))
    val bottomInset = WindowInsets.navigationBarsIgnoringVisibility.getBottom(density)

    BoxWithConstraints(Modifier.fillMaxSize().background(theme.paperColor)) {
        val w = constraints.maxWidth
        val h = constraints.maxHeight
        val fontScale = LocalConfiguration.current.fontScale
        val spec = with(density) {
            PageSpec(
                width = w, height = h, theme = theme, typeface = prefs.typeface,
                style = PageStyle(
                    fontSize = (prefs.fontSizeSp * fontScale * this.density).roundToInt().toUShort(),
                    margin = prefs.margins.dp.dp.roundToPx(),
                    top = topInset + 30.dp.roundToPx(),
                    bottom = bottomInset + 40.dp.roundToPx(),
                    lineSpacing = prefs.spacing.percent,
                    hyphenate = prefs.hyphenate,
                ),
            )
        }
        LaunchedEffect(spec) { controller.configure(spec) }

        val width = w.toFloat()
        suspend fun slide(dir: Int) {
            val target = -dir * width
            animate(offset, target, animationSpec = tween(durationMillis = 210, easing = FastOutSlowInEasing)) { v, _ -> offset = v }
            controller.turn(dir) { offset = 0f }
        }
        fun turn(dir: Int) {
            scope.launch {
                turnLock.withLock {
                    val p = controller.pages
                    val neighbour = if (dir > 0) p.next else p.prev
                    if (neighbour != null) slide(dir) else controller.turn(dir)
                }
            }
        }
        LaunchedEffect(Unit) { volumeTurns.collect { if (sheet == Sheet.None) turn(it) } }

        // --- the page, following the finger during a swipe ---
        Canvas(
            Modifier.fillMaxSize()
                .pointerInput(Unit) {
                    val tracker = VelocityTracker()
                    detectHorizontalDragGestures(
                        onDragStart = { tracker.resetTracking(); controls = false; word = null },
                        onDragEnd = {
                            val v = tracker.calculateVelocity().x
                            scope.launch {
                                turnLock.withLock {
                                    val p = controller.pages
                                    val dir = when {
                                        (offset < -width * 0.18f || v < -900f) && offset < 0 && p.next != null -> 1
                                        (offset > width * 0.18f || v > 900f) && offset > 0 && p.prev != null -> -1
                                        else -> 0
                                    }
                                    if (dir != 0) slide(dir)
                                    else animate(offset, 0f, animationSpec = tween(180)) { x, _ -> offset = x }
                                }
                            }
                        },
                        onDragCancel = { scope.launch { animate(offset, 0f) { x, _ -> offset = x } } },
                    ) { change, dx ->
                        change.consume()
                        tracker.addPosition(change.uptimeMillis, change.position)
                        val p = controller.pages
                        val blocked = (offset + dx < 0 && p.next == null) || (offset + dx > 0 && p.prev == null)
                        offset = (offset + if (blocked) dx * 0.2f else dx).coerceIn(-width, width)
                    }
                }
                .pointerInput(Unit) {
                    detectTapGestures(
                        onTap = { pos ->
                            if (word != null) { word = null; return@detectTapGestures }
                            if (controls) { controls = false; return@detectTapGestures }
                            scope.launch {
                                when (val t = controller.tap(pos.x.toInt(), pos.y.toInt())) {
                                    is TapResult.Jump -> controller.goto(t.position)
                                    is TapResult.Note -> note = t
                                    is TapResult.External -> external = t.url
                                    TapResult.Nothing -> when {
                                        pos.x < width * 0.3f -> turn(-1)
                                        pos.x > width * 0.7f -> turn(1)
                                        else -> controls = true
                                    }
                                }
                            }
                        },
                        onLongPress = { pos ->
                            scope.launch { word = controller.wordAt(pos.x.toInt(), pos.y.toInt()) }
                        },
                    )
                },
        ) {
            val cur = pages.current
            if (cur != null) {
                if (offset < 0) pages.next?.let { drawImage(it, Offset(offset + width, 0f)) }
                if (offset > 0) pages.prev?.let { drawImage(it, Offset(offset - width, 0f)) }
                drawImage(cur, Offset(offset, 0f))
                // a soft shadow on the edge of the moving page
                if (offset != 0f) {
                    val edge = if (offset < 0) offset + width else offset
                    val shade = Color.Black.copy(alpha = if (theme.dark) 0.5f else 0.12f)
                    drawRect(
                        Brush.horizontalGradient(listOf(shade, Color.Transparent), startX = edge, endX = edge + 24.dp.toPx()),
                        topLeft = Offset(edge, 0f), size = Size(24.dp.toPx(), size.height),
                    )
                }
            }
            word?.let { wd ->
                drawRoundRect(
                    color = Color(0xFFE0A27E).copy(alpha = 0.35f),
                    topLeft = Offset(wd.left.toFloat() + offset, wd.top.toFloat()),
                    size = Size((wd.right - wd.left).toFloat(), (wd.bottom - wd.top).toFloat()),
                    cornerRadius = CornerRadius(4.dp.toPx()),
                )
            }
        }

        // --- footer: where you are (in the page's colours, the theme's type) ---
        if (pages.current != null) {
            val faded = theme.inkColor.copy(alpha = 0.55f)
            val footer = TextStyle(fontFamily = look.theme.ui, fontSize = 11.sp, letterSpacing = if (look.theme.caps) 0.6.sp else 0.sp)
            Row(
                Modifier.align(Alignment.BottomCenter).fillMaxWidth()
                    .padding(bottom = with(density) { bottomInset.toDp() } + 12.dp)
                    .padding(horizontal = (prefs.margins.dp + 2).dp)
                    .offset { IntOffset(offset.roundToInt(), 0) },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(look.caps(pages.chapterTitle), style = footer, color = faded, maxLines = 1, overflow = TextOverflow.Ellipsis,
                     modifier = Modifier.weight(1f))
                Spacer(Modifier.width(12.dp))
                val left = pages.pageCount - pages.page - 1
                val where = if (pages.chaptered) " in chapter" else ""
                val pageText = if (left > 0) "$left ${if (left == 1) "page" else "pages"} left$where" else "Last page$where"
                Text(look.caps("$pageText  ·  ${(pages.progress * 100).roundToInt()}%"), style = footer, color = faded, maxLines = 1)
            }
        }
        if (pages.bookmarked) {
            Icon(
                Icons.Filled.Bookmark, contentDescription = "Bookmarked",
                tint = if (theme.dark) theme.inkColor else look.p.accent,
                modifier = Modifier.align(Alignment.TopEnd).padding(top = with(density) { topInset.toDp() }, end = 14.dp)
                    .size(26.dp).offset { IntOffset(offset.roundToInt(), 0) },
            )
        }
        if (pages.current == null && controller.error == null) {
            Text(look.caps("Opening…"), style = Type.button, color = theme.inkColor.copy(alpha = 0.6f), modifier = Modifier.align(Alignment.Center))
        }
        controller.error?.let { msg ->
            Column(Modifier.align(Alignment.Center).padding(32.dp)) {
                Text(look.caps("This couldn't be opened"), style = Type.heading, color = theme.inkColor)
                Spacer(Modifier.height(8.dp))
                Text(msg, style = Type.body, color = theme.inkColor.copy(alpha = 0.7f))
                Spacer(Modifier.height(16.dp))
                Text(look.caps("‹ Back"), style = Type.button, color = theme.inkColor, modifier = Modifier.clickable(onClick = onClose).padding(vertical = 8.dp))
            }
        }

        // --- long-press menu for a word ---
        word?.let { wd ->
            Box(Modifier.offset { IntOffset(wd.left, wd.bottom) }) {
                DropdownMenu(expanded = true, onDismissRequest = { word = null }) {
                    DropdownMenuItem(text = { Text("Define “${wd.text}”") }, onClick = {
                        webSearch(context, "define ${wd.text}"); word = null
                    })
                    DropdownMenuItem(text = { Text("Search in book") }, onClick = {
                        searchQuery = wd.text; sheet = Sheet.Search; word = null
                    })
                    DropdownMenuItem(text = { Text("Copy") }, onClick = {
                        (context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager)
                            .setPrimaryClip(ClipData.newPlainText(wd.text, wd.text))
                        word = null
                    })
                    DropdownMenuItem(text = { Text("Share") }, onClick = {
                        context.startActivity(Intent.createChooser(
                            Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, wd.text), null))
                        word = null
                    })
                }
            }
        }

        // --- first time: show where to tap ---
        if (!prefs.seenTapGuide && pages.current != null && controller.error == null) {
            TapGuide(theme) { settings.update { it.copy(seenTapGuide = true) } }
        }

        // --- controls ---
        AnimatedVisibility(
            visible = controls, enter = fadeIn() + slideInVertically { -it }, exit = fadeOut() + slideOutVertically { -it },
            modifier = Modifier.align(Alignment.TopCenter),
        ) {
            val l = look
            Column(Modifier.fillMaxWidth().background(l.p.paper).windowInsetsPadding(WindowInsets.statusBars)) {
                Row(Modifier.fillMaxWidth().height(60.dp).padding(end = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text("‹", style = Type.display, color = l.p.ink,
                         modifier = Modifier.clickable(onClick = onClose).padding(horizontal = 16.dp, vertical = 6.dp))
                    Column(Modifier.weight(1f).padding(end = 8.dp)) {
                        Text(source.title, style = Type.title.copy(fontSize = 16.sp), color = l.p.ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        if (source.author.isNotBlank()) {
                            Text(l.caps(source.author), style = Type.label, color = l.p.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                    InkButton("Find", { sheet = Sheet.Search }, compact = true)
                    if (source.state.supportsBookmarks) {
                        Spacer(Modifier.width(6.dp))
                        InkButton(if (pages.bookmarked) "Marked" else "Mark", { scope.launch { controller.toggleBookmark() } },
                                  compact = true, filled = pages.bookmarked)
                    }
                }
                Rule(thickness = if (l.theme.id == "eink") 2.dp else 1.dp, color = if (l.theme.id == "eink") l.p.ink else l.p.faint)
            }
        }
        AnimatedVisibility(
            visible = controls, enter = fadeIn() + slideInVertically { it }, exit = fadeOut() + slideOutVertically { it },
            modifier = Modifier.align(Alignment.BottomCenter),
        ) {
            BottomControls(
                pages = pages,
                onSeek = { f -> scope.launch { controller.gotoProgress(f) } },
                onContents = { sheet = Sheet.Contents },
                onSettings = { sheet = Sheet.Settings },
            )
        }
    }

    // --- sheets and dialogs ---
    when (sheet) {
        Sheet.Contents -> ContentsSheet(controller, onDismiss = { sheet = Sheet.None }) { p ->
            sheet = Sheet.None; controls = false
            scope.launch { controller.goto(p) }
        }
        Sheet.Settings -> SettingsSheet(prefs, systemDark, settings::update) { sheet = Sheet.None }
        Sheet.Search -> SearchSheet(controller, searchQuery, { searchQuery = it }, onDismiss = { sheet = Sheet.None }) { r ->
            sheet = Sheet.None; controls = false
            scope.launch { controller.goto(r.position) }
        }
        Sheet.None -> {}
    }
    note?.let { n ->
        InkDialog(
            title = "Note", confirm = "Go there", dismiss = "Close",
            onConfirm = { note = null; scope.launch { controller.goto(n.target) } }, onDismiss = { note = null },
            body = {
                Text(n.text, style = Type.body.copy(fontSize = 15.sp, lineHeight = 22.sp), color = look.p.ink,
                     modifier = Modifier.heightIn(max = 360.dp).verticalScroll(rememberScrollState()))
            },
        )
    }
    external?.let { url ->
        InkDialog(
            title = "Open link", text = url, confirm = "Open", onDismiss = { external = null },
            onConfirm = {
                external = null
                try {
                    context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
                } catch (e: Exception) {
                    Toast.makeText(context, "No app can open this link.", Toast.LENGTH_SHORT).show()
                }
            },
        )
    }
}

/** The three tap zones, shown over the first page once; any tap dismisses it. */
@Composable
private fun TapGuide(theme: PageTheme, onDone: () -> Unit) {
    val l = look
    val veil = if (theme.dark) Color.Black.copy(alpha = 0.92f) else Color(0xFF141414).copy(alpha = 0.93f)
    val fg = Color(0xFFF3F1EC)
    val label = TextStyle(fontFamily = l.theme.label, fontWeight = FontWeight.SemiBold, fontSize = 13.sp,
                          letterSpacing = if (l.theme.caps) 1.3.sp else 0.sp)
    Row(Modifier.fillMaxSize().background(veil).clickable(onClick = onDone)) {
        listOf("‹" to "Back", "☰" to "Menu", "›" to "Next").forEachIndexed { i, (mark, text) ->
            if (i > 0) Box(Modifier.width(1.dp).fillMaxHeight().background(fg.copy(alpha = 0.35f)))
            Column(Modifier.weight(if (i == 1) 0.4f else 0.3f).fillMaxHeight(), verticalArrangement = Arrangement.Center,
                   horizontalAlignment = Alignment.CenterHorizontally) {
                Text(if (mark == "☰") "···" else mark, style = label.copy(fontSize = 34.sp), color = fg)
                Spacer(Modifier.height(10.dp))
                Text(l.caps(text), style = label, color = fg, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
                if (i != 1) {
                    Spacer(Modifier.height(6.dp))
                    Text(l.caps("tap or swipe"), style = label.copy(fontSize = 10.sp, fontWeight = FontWeight.Normal), color = fg.copy(alpha = 0.7f))
                }
            }
        }
    }
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.BottomCenter) {
        Text(l.caps("Tap anywhere to start reading"), style = label.copy(fontSize = 11.sp), color = fg.copy(alpha = 0.8f),
             modifier = Modifier.padding(bottom = 72.dp))
    }
}

private fun webSearch(context: Context, q: String) {
    try {
        context.startActivity(Intent(Intent.ACTION_WEB_SEARCH).putExtra("query", q))
    } catch (e: Exception) {
        context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("https://duckduckgo.com/?q=" + Uri.encode(q))))
    }
}

/** Full-screen reading: bars hidden while reading, shown with the controls. */
@Composable
private fun SystemBars(visible: Boolean, lightBars: Boolean, keepOn: Boolean) {
    val view = LocalView.current
    val window = (view.context as? Activity)?.window ?: return
    DisposableEffect(visible, lightBars, keepOn) {
        val c = WindowCompat.getInsetsController(window, view)
        c.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        if (visible) c.show(WindowInsetsCompat.Type.systemBars()) else c.hide(WindowInsetsCompat.Type.systemBars())
        c.isAppearanceLightStatusBars = lightBars
        c.isAppearanceLightNavigationBars = lightBars
        view.keepScreenOn = keepOn
        onDispose {}
    }
    DisposableEffect(Unit) {
        onDispose {
            val c = WindowCompat.getInsetsController(window, view)
            c.show(WindowInsetsCompat.Type.systemBars())
            view.keepScreenOn = false
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun BottomControls(pages: Pages, onSeek: (Float) -> Unit, onContents: () -> Unit, onSettings: () -> Unit) {
    val l = look
    var dragging by remember { mutableStateOf<Float?>(null) }
    Column(Modifier.fillMaxWidth().background(l.p.paper)) {
        Rule(thickness = if (l.theme.id == "eink") 2.dp else 1.dp, color = if (l.theme.id == "eink") l.p.ink else l.p.faint)
        Column(Modifier.fillMaxWidth().windowInsetsPadding(WindowInsets.navigationBars).padding(horizontal = 16.dp, vertical = 10.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(l.caps(pages.chapterTitle), style = Type.label, color = l.p.ink, maxLines = 1,
                     overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                val pct = ((dragging ?: pages.progress) * 100).roundToInt()
                Text(if (l.theme.id == "eink") "%03d%%".format(pct) else "$pct%", style = Type.meta, color = l.p.ink)
            }
            Slider(
                value = dragging ?: pages.progress,
                onValueChange = { dragging = it },
                onValueChangeFinished = { dragging?.let(onSeek); dragging = null },
                thumb = {
                    Box(Modifier.size(width = if (l.theme.corner > 0.dp) 20.dp else 8.dp, height = 22.dp)
                        .background(l.p.accent, if (l.theme.corner > 0.dp) CircleShape else RectangleShape))
                },
                track = { state ->
                    val f = ((state.value - state.valueRange.start) / (state.valueRange.endInclusive - state.valueRange.start)).coerceIn(0f, 1f)
                    Box(Modifier.fillMaxWidth().height(if (l.theme.id == "eink") 4.dp else 3.dp).background(l.p.faint, l.smallShape)) {
                        Box(Modifier.fillMaxWidth(f).fillMaxHeight().background(l.p.accent, l.smallShape))
                    }
                },
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                InkButton("Contents", onContents, Modifier.weight(1f))
                InkButton("Page", onSettings, Modifier.weight(1f))
            }
        }
    }
}

@Composable
private fun ContentsSheet(controller: ReaderController, onDismiss: () -> Unit, onPick: (uniffi.engine_ffi.Position) -> Unit) {
    val l = look
    var tab by remember { mutableIntStateOf(0) }
    var toc by remember { mutableStateOf<List<TocItem>>(emptyList()) }
    var here by remember { mutableStateOf<uniffi.engine_ffi.Position?>(null) }
    var bookmarks by remember { mutableStateOf(controller.bookmarks()) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(Unit) {
        toc = controller.toc()
        here = controller.position()
    }
    InkSheet(if (tab == 0) "Contents" else "Bookmarks", onDismiss) {
        Box(Modifier.padding(16.dp)) {
            Segmented(listOf(0, 1), tab, { if (it == 0) "Contents" else "Bookmarks (${bookmarks.size})" }) { tab = it }
        }
        if (tab == 0) {
            // the entry covering the current page: the last one starting at or before it
            val current = here?.let { h -> toc.indexOfLast { it.position.chapter <= h.chapter } } ?: -1
            val state = rememberLazyListState(initialFirstVisibleItemIndex = (current - 3).coerceAtLeast(0))
            LazyColumn(state = state, modifier = Modifier.fillMaxWidth().heightIn(max = 560.dp)) {
                itemsIndexed(toc) { i, item ->
                    val on = i == current
                    Row(Modifier.fillMaxWidth().clickable { onPick(item.position) }
                        .background(if (on && l.theme.id == "eink") l.p.ink else Color.Transparent)
                        .padding(horizontal = 16.dp, vertical = 13.dp), verticalAlignment = Alignment.CenterVertically) {
                        if (l.theme.id == "eink") {
                            Text("%02d".format(i + 1), style = Type.meta, color = if (on) l.p.paper else l.p.muted, modifier = Modifier.width(32.dp))
                        }
                        Text(item.title, style = Type.title.copy(fontSize = 16.sp, fontWeight = if (on) FontWeight.Bold else FontWeight.Normal),
                             color = when { on && l.theme.id == "eink" -> l.p.paper; on -> l.p.accent; else -> l.p.ink })
                    }
                    Rule(Modifier.padding(horizontal = 16.dp), color = l.p.faint)
                }
            }
        } else if (bookmarks.isEmpty()) {
            Text("No bookmarks yet. Tap the middle of a page, then MARK.", style = Type.body, color = l.p.muted, modifier = Modifier.padding(16.dp))
        } else {
            LazyColumn(Modifier.fillMaxWidth().heightIn(max = 560.dp)) {
                items(bookmarks) { b ->
                    Row(Modifier.fillMaxWidth().clickable { onPick(b.position) }.padding(start = 16.dp, end = 8.dp, top = 10.dp, bottom = 10.dp),
                        verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(b.label.ifBlank { "Bookmark" }, style = Type.body.copy(fontSize = 15.sp), color = l.p.ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
                            Text(l.caps(java.text.DateFormat.getDateInstance().format(java.util.Date(b.created))), style = Type.label, color = l.p.muted)
                        }
                        InkButton("×", { scope.launch { controller.removeBookmark(b); bookmarks = controller.bookmarks() } }, compact = true)
                    }
                    Rule(Modifier.padding(horizontal = 16.dp), color = l.p.faint)
                }
            }
        }
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun SearchSheet(
    controller: ReaderController,
    query: String,
    onQuery: (String) -> Unit,
    onDismiss: () -> Unit,
    onPick: (SearchResult) -> Unit,
) {
    val l = look
    var results by remember { mutableStateOf<List<SearchResult>?>(null) }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    fun run() {
        if (query.isBlank()) return
        busy = true
        scope.launch { results = controller.search(query.trim()); busy = false }
    }
    LaunchedEffect(Unit) { if (query.isNotBlank()) run() }
    InkSheet("Find in book", onDismiss) {
        InkField(
            value = query, onChange = onQuery, placeholder = "Words to find", prompt = if (l.theme.id == "eink") "›" else "",
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            onSubmit = { run() },
            modifier = Modifier.padding(16.dp),
        )
        val r = results
        when {
            busy -> Box(Modifier.padding(16.dp)) { Working("Searching") }
            r == null -> {}
            r.isEmpty() -> Text("Nothing found for “$query”.", style = Type.body, color = l.p.muted, modifier = Modifier.padding(16.dp))
            else -> {
                SectionLabel("Results", trailing = "${r.size}${if (r.size >= 500) "+" else ""}")
                LazyColumn(Modifier.fillMaxWidth().heightIn(max = 520.dp)) {
                    items(r) { hit ->
                        Column(Modifier.fillMaxWidth().clickable { onPick(hit) }.padding(horizontal = 16.dp, vertical = 12.dp)) {
                            Text(l.caps(hit.chapterTitle), style = Type.label, color = l.p.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Spacer(Modifier.height(3.dp))
                            Text(highlight(hit.snippet, query.trim()), style = Type.body.copy(fontSize = 15.sp), color = l.p.ink, maxLines = 3)
                        }
                        Rule(Modifier.padding(horizontal = 16.dp), color = l.p.faint)
                    }
                }
            }
        }
        Spacer(Modifier.height(24.dp))
    }
}

private fun highlight(text: String, q: String) = buildAnnotatedString {
    var i = 0
    val lower = text.lowercase()
    val ql = q.lowercase()
    while (ql.isNotEmpty()) {
        val j = lower.indexOf(ql, i)
        if (j < 0) break
        append(text.substring(i, j))
        withStyle(SpanStyle(fontWeight = FontWeight.Bold, textDecoration = androidx.compose.ui.text.style.TextDecoration.Underline)) {
            append(text.substring(j, j + ql.length))
        }
        i = j + ql.length
    }
    append(text.substring(i))
}

/** How the page looks: size, typeface, colours, margins, spacing. */
@Composable
private fun SettingsSheet(s: ReaderSettings, systemDark: Boolean, update: ((ReaderSettings) -> ReaderSettings) -> Unit, onDismiss: () -> Unit) {
    val l = look
    InkSheet("Page", onDismiss) {
        Column(Modifier.padding(horizontal = 16.dp).padding(bottom = 32.dp).verticalScroll(rememberScrollState())) {
            SheetLabel("Text size")
            Row(verticalAlignment = Alignment.CenterVertically) {
                InkButton("A −", { update { it.copy(fontSizeSp = (it.fontSizeSp - 1).coerceAtLeast(ReaderSettings.MIN_FONT)) } }, Modifier.weight(1f))
                Text("${s.fontSizeSp}", style = Type.display.copy(fontSize = 20.sp), color = l.p.ink,
                     textAlign = androidx.compose.ui.text.style.TextAlign.Center, modifier = Modifier.width(64.dp))
                InkButton("A +", { update { it.copy(fontSizeSp = (it.fontSizeSp + 1).coerceAtMost(ReaderSettings.MAX_FONT)) } }, Modifier.weight(1f))
            }
            SheetLabel("Typeface")
            Segmented(Typeface.entries, s.typeface, { it.label }) { t -> update { it.copy(typeface = t) } }
            SheetLabel("Page colour")
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                val shown = s.effectiveTheme(systemDark)
                PageTheme.entries.forEach { t ->
                    val on = t == shown
                    Column(Modifier.weight(1f).clickable { update { it.copy(theme = t, autoNight = if (t.dark) false else it.autoNight) } },
                           horizontalAlignment = Alignment.CenterHorizontally) {
                        Box(
                            Modifier.fillMaxWidth().height(56.dp).background(t.paperColor, l.smallShape)
                                .border(if (on) 3.dp else 1.dp, if (on) l.p.accent else l.p.faint, l.smallShape),
                            contentAlignment = Alignment.Center,
                        ) { Text("Aa", color = t.inkColor, fontFamily = FontFamily.Serif, fontSize = 18.sp) }
                        Spacer(Modifier.height(5.dp))
                        Text(l.caps(t.label), style = Type.label, color = if (on) l.p.ink else l.p.muted)
                    }
                }
            }
            CheckRow("Night colours when the phone is in dark mode", null, s.autoNight) { v -> update { it.copy(autoNight = v) } }
            SheetLabel("Margins")
            Segmented(Margins.entries, s.margins, { it.label }) { m -> update { it.copy(margins = m) } }
            SheetLabel("Line spacing")
            Segmented(Spacing.entries, s.spacing, { it.label }) { m -> update { it.copy(spacing = m) } }
            Spacer(Modifier.height(8.dp))
            CheckRow("Hyphenation", null, s.hyphenate) { v -> update { it.copy(hyphenate = v) } }
            CheckRow("Turn pages with volume keys", null, s.volumeKeys) { v -> update { it.copy(volumeKeys = v) } }
            CheckRow("Keep the screen on while reading", null, s.keepScreenOn) { v -> update { it.copy(keepScreenOn = v) } }
        }
    }
}

@Composable
private fun SheetLabel(text: String) {
    Text(look.caps(text), style = Type.label, color = look.p.muted, modifier = Modifier.padding(top = 18.dp, bottom = 8.dp))
}

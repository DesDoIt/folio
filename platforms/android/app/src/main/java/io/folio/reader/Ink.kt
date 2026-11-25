package io.folio.reader

import android.content.Context
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.BottomSheetDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Shapes
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.ColorMatrix
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/*
 * Folio's design system. Every screen is written once against these components; an app theme
 * (AppTheme) supplies the tokens they draw with: palette, type, corners, rules and a few variants.
 */

// --- tokens ---------------------------------------------------------------------------------------

data class Palette(
    val paper: Color,
    val ink: Color,
    /** secondary text */
    val muted: Color,
    /** hairline rules, empty blocks */
    val faint: Color,
    /** a slightly different surface: wells, tonal buttons */
    val well: Color,
    /** the one colour that marks action and selection */
    val accent: Color,
    /** text on the accent */
    val onAccent: Color,
    val dark: Boolean,
)

val Mono = FontFamily(
    Font(R.font.plex_mono_regular, FontWeight.Normal),
    Font(R.font.plex_mono_medium, FontWeight.Medium),
    Font(R.font.plex_mono_semibold, FontWeight.SemiBold),
    Font(R.font.plex_mono_bold, FontWeight.Bold),
)

/** How a theme draws selection in tab bars and segmented controls. */
enum class SelectStyle { Invert, Underline, Pill }

/** How the library lays out books. */
enum class ShelfStyle { List, Grid }

/** An app theme: the look of everything around the page. */
data class AppTheme(
    val id: String,
    val name: String,
    val blurb: String,
    val light: Palette,
    val dark: Palette,
    /** interface text */
    val ui: FontFamily,
    /** section labels and navigation */
    val label: FontFamily,
    /** headers */
    val display: FontFamily,
    /** book and article titles */
    val content: FontFamily,
    val corner: Dp,
    /** outline weight of buttons, fields, panels (0 = filled surfaces instead) */
    val stroke: Dp,
    /** labels in capitals with letter-spacing */
    val caps: Boolean,
    val select: SelectStyle,
    val shelf: ShelfStyle,
    val grayscaleImages: Boolean,
    /** "FOLIO / LIBRARY" path headers instead of a large title */
    val pathHeader: Boolean,
    /** needs a supporter unlock (Play Billing, later) */
    val supporter: Boolean = false,
)

object Themes {
    val Eink = AppTheme(
        id = "eink", name = "E-ink", blurb = "Ink on paper, ruled and square. Where Folio comes from.",
        light = Palette(Color(0xFFF3F1EC), Color(0xFF141414), Color(0xFF5F5D58), Color(0xFFCBC7BE), Color(0xFFE9E6DF),
                        Color(0xFF141414), Color(0xFFF3F1EC), false),
        dark = Palette(Color(0xFF121212), Color(0xFFE9E6E0), Color(0xFF9B9890), Color(0xFF3A3936), Color(0xFF1C1C1B),
                       Color(0xFFE9E6E0), Color(0xFF121212), true),
        ui = Mono, label = Mono, display = Mono, content = FontFamily.Serif,
        corner = 0.dp, stroke = 1.5.dp, caps = true, select = SelectStyle.Invert, shelf = ShelfStyle.List,
        grayscaleImages = true, pathHeader = true,
    )
    val Editorial = AppTheme(
        id = "editorial", name = "Editorial", blurb = "Like a good bookshop and a quality newspaper.",
        light = Palette(Color(0xFFF8F4EC), Color(0xFF1E1B18), Color(0xFF6B635A), Color(0xFFD8CFC2), Color(0xFFEFE8DC),
                        Color(0xFF7A1F1F), Color(0xFFF8F4EC), false),
        dark = Palette(Color(0xFF16130F), Color(0xFFEDE6DA), Color(0xFFA2988B), Color(0xFF3B352E), Color(0xFF211D18),
                       Color(0xFFD98C80), Color(0xFF16130F), true),
        ui = FontFamily.Serif, label = FontFamily.Serif, display = FontFamily.Serif, content = FontFamily.Serif,
        corner = 0.dp, stroke = 1.dp, caps = true, select = SelectStyle.Underline, shelf = ShelfStyle.Grid,
        grayscaleImages = false, pathHeader = false,
    )
    val Warm = AppTheme(
        id = "warm", name = "Warm", blurb = "Soft, rounded and friendly.",
        light = Palette(Color(0xFFFFFAF4), Color(0xFF2B211B), Color(0xFF786A5F), Color(0xFFEADFD3), Color(0xFFF5EADF),
                        Color(0xFFC4553A), Color(0xFFFFFFFF), false),
        dark = Palette(Color(0xFF1B1613), Color(0xFFF3E9E0), Color(0xFFB3A497), Color(0xFF3A302A), Color(0xFF29221D),
                       Color(0xFFF08E6F), Color(0xFF2B160F), true),
        ui = FontFamily.SansSerif, label = FontFamily.SansSerif, display = FontFamily.SansSerif, content = FontFamily.Serif,
        corner = 16.dp, stroke = 0.dp, caps = false, select = SelectStyle.Pill, shelf = ShelfStyle.Grid,
        grayscaleImages = false, pathHeader = false,
    )
    val all = listOf(Eink, Editorial, Warm)

    fun byId(id: String?) = all.firstOrNull { it.id == id } ?: Eink
}

/** What the reader is allowed to use. Every theme is free for now; Play Billing plugs in here later. */
object Entitlements {
    fun unlocked(t: AppTheme): Boolean = !t.supporter
}

enum class DarkMode(val label: String) { System("System"), Light("Light"), Dark("Dark") }

class AppearanceStore private constructor(context: Context) {
    private val prefs = context.getSharedPreferences("appearance", Context.MODE_PRIVATE)
    private val _theme = MutableStateFlow(Themes.byId(prefs.getString("theme", null)))
    val theme: StateFlow<AppTheme> = _theme
    private val _mode = MutableStateFlow(DarkMode.entries.firstOrNull { it.name == prefs.getString("mode", null) } ?: DarkMode.System)
    val mode: StateFlow<DarkMode> = _mode

    fun setTheme(t: AppTheme) {
        if (!Entitlements.unlocked(t)) return
        _theme.value = t
        prefs.edit().putString("theme", t.id).apply()
    }

    fun setMode(m: DarkMode) {
        _mode.value = m
        prefs.edit().putString("mode", m.name).apply()
    }

    companion object {
        @Volatile private var instance: AppearanceStore? = null
        fun get(context: Context): AppearanceStore =
            instance ?: synchronized(this) { instance ?: AppearanceStore(context.applicationContext).also { instance = it } }
    }
}

/** The active theme and its palette for the current light/dark mode. */
data class Look(val theme: AppTheme, val p: Palette) {
    // a 0 dp corner is a square; Material's Shapes needs corner-based shapes either way
    val shape: androidx.compose.foundation.shape.CornerBasedShape get() = RoundedCornerShape(theme.corner)
    val smallShape: androidx.compose.foundation.shape.CornerBasedShape get() = RoundedCornerShape(theme.corner / 2)
    val imageFilter: ColorFilter? get() = if (theme.grayscaleImages) Grayscale else null
    fun caps(s: String) = if (theme.caps) s.uppercase() else s
}

val LocalLook = staticCompositionLocalOf { Look(Themes.Eink, Themes.Eink.light) }

/** The current look. */
val look: Look @Composable get() = LocalLook.current

/** The current palette. */
val ink: Palette @Composable get() = LocalLook.current.p

val Grayscale = ColorFilter.colorMatrix(ColorMatrix().apply { setToSaturation(0f) })

/** Text styles of the current theme. */
object Type {
    val label: TextStyle @Composable get() = look.let {
        TextStyle(fontFamily = it.theme.label, fontWeight = FontWeight.Medium, fontSize = if (it.theme.caps) 11.sp else 13.sp,
                  letterSpacing = if (it.theme.caps) 1.3.sp else 0.1.sp)
    }
    val meta: TextStyle @Composable get() = TextStyle(fontFamily = look.theme.ui, fontSize = 12.sp, letterSpacing = 0.2.sp)
    val ui: TextStyle @Composable get() = TextStyle(fontFamily = look.theme.ui, fontSize = 15.sp)
    val heading: TextStyle @Composable get() = look.let {
        TextStyle(fontFamily = it.theme.display, fontWeight = FontWeight.SemiBold, fontSize = 15.sp,
                  letterSpacing = if (it.theme.caps) 2.sp else 0.sp)
    }
    val display: TextStyle @Composable get() = look.let {
        when (it.theme.id) {
            "editorial" -> TextStyle(fontFamily = FontFamily.Serif, fontWeight = FontWeight.Bold, fontSize = 30.sp, lineHeight = 34.sp)
            "warm" -> TextStyle(fontFamily = FontFamily.SansSerif, fontWeight = FontWeight.Bold, fontSize = 30.sp, lineHeight = 36.sp)
            else -> TextStyle(fontFamily = Mono, fontWeight = FontWeight.SemiBold, fontSize = 22.sp, letterSpacing = 1.sp)
        }
    }
    val title: TextStyle @Composable get() = TextStyle(fontFamily = look.theme.content, fontWeight = FontWeight.Medium, fontSize = 17.sp, lineHeight = 22.sp)
    val body: TextStyle @Composable get() = TextStyle(fontFamily = look.theme.content, fontSize = 14.sp, lineHeight = 20.sp)
    val button: TextStyle @Composable get() = look.let {
        if (it.theme.caps) TextStyle(fontFamily = it.theme.label, fontWeight = FontWeight.SemiBold, fontSize = 12.sp, letterSpacing = 1.3.sp)
        else TextStyle(fontFamily = it.theme.label, fontWeight = FontWeight.SemiBold, fontSize = 14.sp)
    }
}

@Composable
fun FolioTheme(content: @Composable () -> Unit) {
    val store = AppearanceStore.get(LocalContext.current)
    val theme by store.theme.collectAsState()
    val mode by store.mode.collectAsState()
    val dark = when (mode) { DarkMode.System -> isSystemInDarkTheme(); DarkMode.Light -> false; DarkMode.Dark -> true }
    val p = if (dark) theme.dark else theme.light
    val common = { s: androidx.compose.material3.ColorScheme -> s }
    val scheme = (if (dark) darkColorScheme(
        primary = p.accent, onPrimary = p.onAccent, primaryContainer = p.well, onPrimaryContainer = p.ink,
        secondary = p.accent, onSecondary = p.onAccent, secondaryContainer = p.well, onSecondaryContainer = p.ink,
        tertiary = p.accent, background = p.paper, onBackground = p.ink, surface = p.paper, onSurface = p.ink,
        surfaceVariant = p.well, onSurfaceVariant = p.muted, outline = p.ink, outlineVariant = p.faint,
        surfaceContainer = p.paper, surfaceContainerLow = p.paper, surfaceContainerHigh = p.well,
        surfaceContainerHighest = p.well, surfaceContainerLowest = p.paper, error = p.accent, onError = p.onAccent,
        inverseSurface = p.ink, inverseOnSurface = p.paper,
    ) else lightColorScheme(
        primary = p.accent, onPrimary = p.onAccent, primaryContainer = p.well, onPrimaryContainer = p.ink,
        secondary = p.accent, onSecondary = p.onAccent, secondaryContainer = p.well, onSecondaryContainer = p.ink,
        tertiary = p.accent, background = p.paper, onBackground = p.ink, surface = p.paper, onSurface = p.ink,
        surfaceVariant = p.well, onSurfaceVariant = p.muted, outline = p.ink, outlineVariant = p.faint,
        surfaceContainer = p.paper, surfaceContainerLow = p.paper, surfaceContainerHigh = p.well,
        surfaceContainerHighest = p.well, surfaceContainerLowest = p.paper, error = p.accent, onError = p.onAccent,
        inverseSurface = p.ink, inverseOnSurface = p.paper,
    )).let(common)
    val base = Typography()
    val f = theme.ui
    val typography = Typography(
        displayLarge = base.displayLarge.copy(fontFamily = f), displayMedium = base.displayMedium.copy(fontFamily = f),
        displaySmall = base.displaySmall.copy(fontFamily = f), headlineLarge = base.headlineLarge.copy(fontFamily = f),
        headlineMedium = base.headlineMedium.copy(fontFamily = f), headlineSmall = base.headlineSmall.copy(fontFamily = f),
        titleLarge = base.titleLarge.copy(fontFamily = f), titleMedium = base.titleMedium.copy(fontFamily = f),
        titleSmall = base.titleSmall.copy(fontFamily = f), bodyLarge = base.bodyLarge.copy(fontFamily = f),
        bodyMedium = base.bodyMedium.copy(fontFamily = f), bodySmall = base.bodySmall.copy(fontFamily = f),
        labelLarge = base.labelLarge.copy(fontFamily = f), labelMedium = base.labelMedium.copy(fontFamily = f),
        labelSmall = base.labelSmall.copy(fontFamily = f),
    )
    val l = Look(theme, p)
    val shapes = Shapes(l.smallShape, l.smallShape, l.shape, l.shape, l.shape)
    CompositionLocalProvider(LocalLook provides l) {
        MaterialTheme(colorScheme = scheme, typography = typography, shapes = shapes, content = content)
    }
}

// --- structure ------------------------------------------------------------------------------------

@Composable
fun Rule(modifier: Modifier = Modifier, thickness: Dp = 1.dp, color: Color = ink.ink) {
    Box(modifier.fillMaxWidth().height(thickness).background(color))
}

@Composable
fun VRule(thickness: Dp = 1.dp, color: Color = ink.ink) {
    Box(Modifier.width(thickness).fillMaxHeight().background(color))
}

/**
 * The screen header. E-ink: "FOLIO / LIBRARY" over a heavy rule. Editorial: a large serif title
 * over a double rule. Warm: a large friendly title.
 */
@Composable
fun Header(path: List<String>, onBack: (() -> Unit)? = null, actions: @Composable RowScope.() -> Unit = {}) {
    val l = look
    val p = l.p
    Column(Modifier.fillMaxWidth().background(p.paper).windowInsetsPadding(WindowInsets.statusBars)) {
        if (l.theme.pathHeader) {
            Row(
                Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(start = if (onBack != null) 4.dp else 16.dp, end = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (onBack != null) {
                    Text("‹", style = Type.display, color = p.ink,
                         modifier = Modifier.clickable(onClick = onBack).padding(horizontal = 12.dp, vertical = 4.dp))
                }
                Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
                    path.forEachIndexed { i, part ->
                        if (i > 0) Text(" / ", style = Type.heading, color = p.muted)
                        Text(part.uppercase(), style = Type.heading, color = if (i == path.lastIndex) p.ink else p.muted,
                             maxLines = 1, overflow = TextOverflow.Ellipsis,
                             modifier = if (i == path.lastIndex) Modifier.weight(1f, false) else Modifier)
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically, content = actions)
            }
            Rule(thickness = 2.dp)
        } else {
            Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(start = 4.dp, end = 8.dp, top = 4.dp),
                verticalAlignment = Alignment.CenterVertically) {
                if (onBack != null) {
                    Text("‹  " + (path.getOrNull(path.size - 2) ?: ""), style = Type.label, color = p.accent,
                         modifier = Modifier.clickable(onClick = onBack).padding(horizontal = 12.dp, vertical = 10.dp))
                }
                Spacer(Modifier.weight(1f))
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically, content = actions)
            }
            Text(path.last(), style = Type.display, color = p.ink, maxLines = 1, overflow = TextOverflow.Ellipsis,
                 modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = if (l.theme.id == "editorial") 10.dp else 14.dp))
            if (l.theme.id == "editorial") {
                Rule(Modifier.padding(horizontal = 16.dp), thickness = 2.dp)
                Spacer(Modifier.height(2.dp))
                Rule(Modifier.padding(horizontal = 16.dp), thickness = 0.75.dp)
            }
        }
    }
}

/** A quiet section label (Warm: a bold heading), with an optional count or action on the right. */
@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier, trailing: String? = null, inset: Dp = 16.dp) {
    val l = look
    Row(modifier.fillMaxWidth().padding(horizontal = inset, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
        if (l.theme.id == "warm") {
            Text(text, style = TextStyle(fontFamily = l.theme.display, fontWeight = FontWeight.Bold, fontSize = 19.sp), color = l.p.ink,
                 modifier = Modifier.weight(1f))
            trailing?.let { Text(it, style = Type.meta, color = l.p.muted) }
        } else {
            Text(l.caps(text), style = Type.label, color = if (l.theme.id == "editorial") l.p.accent else l.p.muted)
            Spacer(Modifier.weight(1f))
            trailing?.let { Text(l.caps(it), style = Type.label, color = l.p.muted) }
        }
    }
}

/** The bottom navigation, drawn in the theme's selection style. */
@Composable
fun TabBar(labels: List<String>, selected: Int, onSelect: (Int) -> Unit) {
    val l = look
    val p = l.p
    Column(Modifier.fillMaxWidth().background(p.paper)) {
        when (l.theme.select) {
            SelectStyle.Invert -> Rule(thickness = 2.dp)
            SelectStyle.Underline -> Rule(color = p.faint)
            SelectStyle.Pill -> {}
        }
        Row(Modifier.fillMaxWidth().height(if (l.theme.select == SelectStyle.Pill) 64.dp else 52.dp)) {
            labels.forEachIndexed { i, label ->
                if (i > 0 && l.theme.select == SelectStyle.Invert) VRule()
                val on = i == selected
                Box(
                    Modifier.weight(1f).fillMaxHeight()
                        .background(if (on && l.theme.select == SelectStyle.Invert) p.ink else p.paper)
                        .clickable { onSelect(i) },
                    contentAlignment = Alignment.Center,
                ) {
                    when (l.theme.select) {
                        SelectStyle.Invert -> Text(label.uppercase(), style = Type.button, color = if (on) p.paper else p.ink)
                        SelectStyle.Underline -> Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            Text(l.caps(label), style = Type.button, color = if (on) p.ink else p.muted)
                            Spacer(Modifier.height(5.dp))
                            Box(Modifier.size(width = 24.dp, height = 2.dp).background(if (on) p.accent else Color.Transparent))
                        }
                        SelectStyle.Pill -> Text(label, style = Type.button, color = if (on) p.onAccent else p.muted,
                            modifier = Modifier.background(if (on) p.accent else Color.Transparent, CircleShape)
                                .padding(horizontal = 18.dp, vertical = 9.dp))
                    }
                }
            }
        }
        Spacer(Modifier.windowInsetsPadding(WindowInsets.navigationBars))
    }
}

// --- controls -------------------------------------------------------------------------------------

/** A header action as an icon (keeps headers quiet). */
@Composable
fun HeaderIcon(icon: androidx.compose.ui.graphics.vector.ImageVector, description: String, onClick: () -> Unit, highlighted: Boolean = false) {
    val l = look
    androidx.compose.material3.IconButton(onClick = onClick) {
        androidx.compose.material3.Icon(icon, contentDescription = description, tint = if (highlighted) l.p.accent else l.p.ink)
    }
}

/**
 * A compact "Label ↓" that opens a menu: for choices that change rarely (sort order, book source),
 * so they don't take a row of their own. [extras] are actions listed after the options.
 */
@Composable
fun <T> Picker(
    selected: T,
    options: List<T>,
    label: (T) -> String,
    onSelect: (T) -> Unit,
    modifier: Modifier = Modifier,
    prefix: String? = null,
    extras: List<Pair<String, () -> Unit>> = emptyList(),
) {
    val l = look
    var open by remember { androidx.compose.runtime.mutableStateOf(false) }
    Box(modifier) {
        Row(Modifier.clickable { open = true }.padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            prefix?.let { Text(l.caps(it) + " ", style = Type.label, color = l.p.muted) }
            Text(l.caps(label(selected)), style = Type.button, color = l.p.ink, maxLines = 1, overflow = TextOverflow.Ellipsis,
                 modifier = Modifier.weight(1f, fill = false))
            Text(" ↓", style = Type.button, color = l.p.accent)
        }
        androidx.compose.material3.DropdownMenu(expanded = open, onDismissRequest = { open = false }, containerColor = l.p.paper,
                                               modifier = if (l.theme.id == "eink") Modifier.border(1.5.dp, l.p.ink, RectangleShape) else Modifier) {
            options.forEach { o ->
                androidx.compose.material3.DropdownMenuItem(
                    text = { Text(label(o), style = Type.ui.copy(fontWeight = if (o == selected) FontWeight.Bold else FontWeight.Normal), color = l.p.ink) },
                    onClick = { open = false; onSelect(o) },
                )
            }
            extras.forEach { (text, action) ->
                androidx.compose.material3.DropdownMenuItem(
                    text = { Text(text, style = Type.ui, color = l.p.accent) },
                    onClick = { open = false; action() },
                )
            }
        }
    }
}

/** A button in the theme's style. [filled] marks the main action. */
@Composable
fun InkButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    filled: Boolean = false,
    enabled: Boolean = true,
    compact: Boolean = false,
) {
    val l = look
    val p = l.p
    val src = remember { MutableInteractionSource() }
    val pressed by src.collectIsPressedAsState()
    val outlined = l.theme.stroke > 0.dp
    // E-ink inverts on press; the others darken slightly
    val solid = if (l.theme.select == SelectStyle.Invert) filled != pressed else filled
    val bg = when {
        !enabled -> Color.Transparent
        solid -> p.accent
        !outlined -> p.well
        pressed -> p.well
        else -> Color.Transparent
    }
    val fg = when { !enabled -> p.muted; solid -> p.onAccent; outlined -> p.ink; else -> p.ink }
    Box(
        modifier
            .then(if (outlined) Modifier.border(BorderStroke(l.theme.stroke, if (enabled) (if (solid) p.accent else p.ink) else p.faint), l.smallShape) else Modifier)
            .background(bg, l.smallShape)
            .clickable(interactionSource = src, indication = null, enabled = enabled, onClick = onClick)
            .padding(horizontal = if (compact) 10.dp else 16.dp, vertical = if (compact) 6.dp else 10.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(l.caps(text), style = Type.button, color = fg, maxLines = 1)
    }
}

/** A button that fills left to right as a download progresses; tapping it cancels. */
@Composable
fun ProgressButton(fraction: Float?, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val l = look
    val p = l.p
    var tick by remember { mutableIntStateOf(0) }
    if (fraction == null) LaunchedEffect(Unit) { while (true) { delay(350); tick++ } }
    Box(
        modifier.then(if (l.theme.stroke > 0.dp) Modifier.border(BorderStroke(l.theme.stroke, p.accent), l.smallShape) else Modifier)
            .background(p.well, l.smallShape).clickable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Canvas(Modifier.matchParentSize()) {
            val f = fraction ?: 0.25f
            val x = if (fraction == null) size.width * ((tick % 4) / 4f) else 0f
            drawRect(p.accent, topLeft = Offset(x, 0f), size = size.copy(width = size.width * f))
        }
        Text(fraction?.let { "${(it * 100).toInt()}%" } ?: "···", style = Type.button,
             color = if ((fraction ?: 0f) > 0.55f) p.onAccent else p.ink,
             modifier = Modifier.padding(horizontal = 14.dp, vertical = 9.dp))
    }
}

private fun Modifier.matchParentSize(): Modifier = this.then(Modifier.fillMaxSize())

/** One choice among several, in the theme's selection style. Scrolls sideways when [scroll] is set. */
@Composable
fun <T> Segmented(options: List<T>, selected: T?, label: (T) -> String, modifier: Modifier = Modifier, scroll: Boolean = false, onSelect: (T) -> Unit) {
    val l = look
    val p = l.p
    val row = if (scroll) modifier.horizontalScroll(rememberScrollState()) else modifier.fillMaxWidth()
    when (l.theme.select) {
        SelectStyle.Invert -> Row(row.height(IntrinsicSize.Min).border(BorderStroke(l.theme.stroke, p.ink), RectangleShape)) {
            options.forEachIndexed { i, o ->
                if (i > 0) VRule(l.theme.stroke)
                val on = o == selected
                Box(
                    (if (scroll) Modifier else Modifier.weight(1f)).background(if (on) p.ink else Color.Transparent)
                        .clickable { onSelect(o) }.padding(horizontal = 12.dp, vertical = 9.dp),
                    contentAlignment = Alignment.Center,
                ) { Text(label(o).uppercase(), style = Type.button, color = if (on) p.paper else p.ink, maxLines = 1) }
            }
        }
        SelectStyle.Underline -> Row(row, horizontalArrangement = Arrangement.spacedBy(if (scroll) 18.dp else 0.dp)) {
            options.forEach { o ->
                val on = o == selected
                Column(
                    (if (scroll) Modifier else Modifier.weight(1f)).clickable { onSelect(o) }.padding(vertical = 8.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text(l.caps(label(o)), style = Type.button, color = if (on) p.ink else p.muted, maxLines = 1)
                    Spacer(Modifier.height(5.dp))
                    Box(Modifier.fillMaxWidth().height(2.dp).background(if (on) p.accent else p.faint))
                }
            }
        }
        SelectStyle.Pill -> Row(row, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            options.forEach { o ->
                val on = o == selected
                Box(
                    (if (scroll) Modifier else Modifier.weight(1f)).background(if (on) p.accent else p.well, CircleShape)
                        .clickable { onSelect(o) }.padding(horizontal = 14.dp, vertical = 9.dp),
                    contentAlignment = Alignment.Center,
                ) { Text(label(o), style = Type.button, color = if (on) p.onAccent else p.ink, maxLines = 1) }
            }
        }
    }
}

/** An on/off row: square box for E-ink and Editorial, a switch for Warm. */
@Composable
fun CheckRow(text: String, sub: String? = null, value: Boolean, enabled: Boolean = true, onChange: (Boolean) -> Unit) {
    val l = look
    val p = l.p
    Row(
        Modifier.fillMaxWidth().clickable(enabled = enabled) { onChange(!value) }.padding(vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f).padding(end = 16.dp)) {
            Text(text, style = Type.ui, color = if (enabled) p.ink else p.muted)
            sub?.let { Text(it, style = Type.meta, color = p.muted) }
        }
        if (l.theme.stroke == 0.dp) {
            Switch(checked = value, onCheckedChange = onChange, enabled = enabled,
                   colors = SwitchDefaults.colors(checkedTrackColor = p.accent, checkedThumbColor = p.onAccent,
                                                  uncheckedTrackColor = p.well, uncheckedBorderColor = p.faint, uncheckedThumbColor = p.muted))
        } else {
            Box(Modifier.size(22.dp).border(BorderStroke(l.theme.stroke, if (enabled) p.ink else p.faint), RectangleShape).padding(4.dp)) {
                if (value) Box(Modifier.fillMaxSize().background(if (enabled) p.accent else p.faint))
            }
        }
    }
}

/** Reading progress: blocks (E-ink), a hairline (Editorial) or a rounded bar (Warm). */
@Composable
fun ProgressMeter(fraction: Float, modifier: Modifier = Modifier, showPercent: Boolean = true) {
    val l = look
    val p = l.p
    val pct = (fraction * 100).toInt().coerceIn(0, 100)
    Row(modifier, verticalAlignment = Alignment.CenterVertically) {
        when (l.theme.id) {
            "eink" -> {
                val filled = (fraction * 10).toInt().coerceIn(0, 10).let { if (fraction > 0f && it == 0) 1 else it }
                repeat(10) { i ->
                    Box(Modifier.padding(end = 2.dp).size(width = 7.dp, height = 11.dp)
                        .then(if (i < filled) Modifier.background(p.ink) else Modifier.border(1.dp, p.faint, RectangleShape)))
                }
            }
            else -> {
                val h = if (l.theme.id == "warm") 6.dp else 2.dp
                Box(Modifier.weight(1f, fill = true).height(h).background(p.faint, CircleShape)) {
                    Box(Modifier.fillMaxWidth(fraction.coerceIn(0f, 1f)).fillMaxHeight().background(p.accent, CircleShape))
                }
            }
        }
        if (showPercent) {
            Spacer(Modifier.width(8.dp))
            Text(if (l.theme.id == "eink") "%03d%%".format(pct) else "$pct%", style = Type.meta, color = p.ink)
        }
    }
}

/** A text field in the theme's style. */
@Composable
fun InkField(
    value: String,
    onChange: (String) -> Unit,
    placeholder: String,
    modifier: Modifier = Modifier,
    prompt: String = "›",
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default,
    keyboardActions: KeyboardActions = KeyboardActions.Default,
    visual: VisualTransformation = VisualTransformation.None,
    trailing: @Composable (() -> Unit)? = null,
    /** what Enter does: the keyboard's action key, and also Enter on a hardware keyboard */
    onSubmit: (() -> Unit)? = null,
) {
    val l = look
    val p = l.p
    val actions = if (onSubmit != null) KeyboardActions(onAny = { onSubmit() }) else keyboardActions
    Row(
        modifier.fillMaxWidth()
            .then(if (l.theme.stroke > 0.dp) Modifier.border(BorderStroke(l.theme.stroke, p.ink), l.smallShape) else Modifier.background(p.well, l.shape))
            .heightIn(min = 48.dp).padding(start = 14.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (prompt.isNotEmpty()) {
            Text(prompt, style = Type.ui.copy(fontWeight = FontWeight.SemiBold), color = p.muted)
            Spacer(Modifier.width(10.dp))
        }
        Box(Modifier.weight(1f).padding(vertical = 12.dp)) {
            if (value.isEmpty()) Text(placeholder, style = Type.ui, color = p.muted, maxLines = 1)
            BasicTextField(
                value = value, onValueChange = onChange, singleLine = true, textStyle = Type.ui.copy(color = p.ink),
                cursorBrush = SolidColor(p.accent), keyboardOptions = keyboardOptions, keyboardActions = actions,
                visualTransformation = visual,
                // a hardware Enter would otherwise move focus to the next control and press it
                modifier = Modifier.fillMaxWidth().onPreviewKeyEvent { e ->
                    val enter = e.key == androidx.compose.ui.input.key.Key.Enter || e.key == androidx.compose.ui.input.key.Key.NumPadEnter
                    if (onSubmit != null && enter) {
                        if (e.type == androidx.compose.ui.input.key.KeyEventType.KeyUp) onSubmit()
                        true
                    } else {
                        false
                    }
                },
            )
        }
        trailing?.invoke()
    }
}

/** Something is loading: an e-reader-style "LOADING ▮▯▯", or a spinner. */
@Composable
fun Working(text: String = "Loading", modifier: Modifier = Modifier) {
    val l = look
    if (l.theme.id == "eink") {
        var tick by remember { mutableIntStateOf(0) }
        LaunchedEffect(Unit) { while (true) { delay(300); tick++ } }
        // drawn, not typed: the mono face has no block glyphs
        Row(modifier, verticalAlignment = Alignment.CenterVertically) {
            if (text.isNotEmpty()) {
                Text(text.uppercase(), style = Type.button, color = l.p.ink)
                Spacer(Modifier.width(10.dp))
            }
            repeat(3) { i ->
                Box(Modifier.padding(end = 3.dp).size(width = 7.dp, height = 12.dp)
                    .then(if (i == tick % 3) Modifier.background(l.p.ink) else Modifier.border(1.dp, l.p.ink, RectangleShape)))
            }
        }
    } else {
        CircularProgressIndicator(modifier.size(28.dp), color = l.p.accent, strokeWidth = 2.5.dp)
    }
}

/** A grouped area: a bordered box, or a soft filled card for Warm. */
@Composable
fun Panel(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    val l = look
    Box(
        modifier.fillMaxWidth().then(
            if (l.theme.stroke > 0.dp) Modifier.border(BorderStroke(l.theme.stroke, l.p.ink), l.shape)
            else Modifier.background(l.p.well, l.shape),
        ),
    ) { content() }
}

/** Small tags: "NEW", "DONE". */
@Composable
fun Tag(text: String, inverted: Boolean = false) {
    val l = look
    val p = l.p
    Text(l.caps(text), style = Type.label.copy(fontSize = 10.sp, fontWeight = FontWeight.SemiBold),
         color = if (inverted) p.onAccent else p.accent,
         modifier = Modifier.background(if (inverted) p.accent else Color.Transparent, l.smallShape)
             .then(if (l.theme.stroke > 0.dp) Modifier.border(1.dp, p.accent, l.smallShape) else Modifier)
             .padding(horizontal = 6.dp, vertical = 1.dp))
}

// --- overlays -------------------------------------------------------------------------------------

/** A sheet from the bottom, with a title and a close action. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun InkSheet(title: String, onDismiss: () -> Unit, content: @Composable () -> Unit) {
    val l = look
    val p = l.p
    val rounded = l.theme.corner > 0.dp
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        shape = if (rounded) RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp) else RectangleShape,
        containerColor = p.paper, contentColor = p.ink, tonalElevation = 0.dp,
        dragHandle = if (rounded) ({ BottomSheetDefaults.DragHandle(color = p.faint) }) else null,
    ) {
        if (!rounded) Rule(thickness = if (l.theme.id == "eink") 2.dp else 1.dp)
        Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 6.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(l.caps(title), style = if (rounded) Type.title.copy(fontFamily = l.theme.display, fontWeight = FontWeight.Bold, fontSize = 20.sp) else Type.heading,
                 color = p.ink, modifier = Modifier.weight(1f))
            Text(l.caps("Close") + if (l.theme.id == "eink") " ×" else "", style = Type.button, color = p.accent,
                 modifier = Modifier.clickable(onClick = onDismiss).padding(10.dp))
        }
        if (!rounded) Rule(color = if (l.theme.id == "eink") p.ink else p.faint)
        content()
    }
}

/** A dialog in the theme's style. */
@Composable
fun InkDialog(
    title: String,
    text: String? = null,
    confirm: String,
    onConfirm: () -> Unit,
    dismiss: String? = "Cancel",
    onDismiss: () -> Unit,
    body: (@Composable () -> Unit)? = null,
) {
    val l = look
    val p = l.p
    AlertDialog(
        onDismissRequest = onDismiss,
        modifier = if (l.theme.id == "eink") Modifier.border(2.dp, p.ink, RectangleShape) else Modifier,
        shape = if (l.theme.corner > 0.dp) RoundedCornerShape(28.dp) else RectangleShape,
        containerColor = p.paper, tonalElevation = 0.dp,
        title = { Text(l.caps(title), style = if (l.theme.caps) Type.heading else Type.title.copy(fontFamily = l.theme.display, fontWeight = FontWeight.Bold, fontSize = 20.sp), color = p.ink) },
        text = body ?: text?.let { { Text(it, style = Type.body.copy(fontSize = 15.sp), color = p.ink) } },
        confirmButton = { InkButton(confirm, onConfirm, filled = true) },
        dismissButton = dismiss?.let { { InkButton(it, onDismiss) } },
    )
}

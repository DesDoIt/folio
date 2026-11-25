package io.folio.reader

import android.content.Context
import androidx.compose.ui.graphics.Color
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import java.io.File

/** A page colour scheme. */
enum class PageTheme(val label: String, val paper: Long, val ink: Long, val dark: Boolean) {
    Paper("Paper", 0xFFF3F1EC, 0xFF161616, false),
    Sepia("Sepia", 0xFFF3E7D3, 0xFF4A3B2C, false),
    Night("Night", 0xFF1C1B1A, 0xFFD9D3CB, true),
    Black("Black", 0xFF000000, 0xFFBDBDBD, true);

    val paperColor get() = Color(paper)
    val inkColor get() = Color(ink)
    val paperRgb get() = (paper and 0xFFFFFF).toUInt()
    val inkRgb get() = (ink and 0xFFFFFF).toUInt()
}

enum class Typeface(val label: String) { Serif("Serif"), Sans("Sans") }

enum class Margins(val label: String, val dp: Int) { Narrow("Narrow", 14), Normal("Normal", 26), Wide("Wide", 42) }

enum class Spacing(val label: String, val percent: Int) { Compact("Compact", 135), Normal("Normal", 150), Relaxed("Relaxed", 170) }

data class ReaderSettings(
    val fontSizeSp: Int = 19,
    val typeface: Typeface = Typeface.Serif,
    val theme: PageTheme = PageTheme.Paper,
    /** follow the system: Night when the phone is in dark mode */
    val autoNight: Boolean = true,
    val margins: Margins = Margins.Normal,
    val spacing: Spacing = Spacing.Normal,
    val hyphenate: Boolean = true,
    val volumeKeys: Boolean = true,
    val keepScreenOn: Boolean = true,
    /** the first-book guide to the tap zones has been shown */
    val seenTapGuide: Boolean = false,
) {
    fun effectiveTheme(systemDark: Boolean) = if (autoNight && systemDark && !theme.dark) PageTheme.Night else theme

    companion object {
        const val MIN_FONT = 12
        const val MAX_FONT = 36
    }
}

class SettingsStore(context: Context) {
    private val prefs = context.getSharedPreferences("reader", Context.MODE_PRIVATE)
    private val _settings = MutableStateFlow(read())
    val settings: StateFlow<ReaderSettings> = _settings

    private fun read(): ReaderSettings {
        val d = ReaderSettings()
        return ReaderSettings(
            fontSizeSp = prefs.getInt("fontSize", d.fontSizeSp).coerceIn(ReaderSettings.MIN_FONT, ReaderSettings.MAX_FONT),
            typeface = enumOr(prefs.getString("typeface", null), d.typeface),
            theme = enumOr(prefs.getString("theme", null), d.theme),
            autoNight = prefs.getBoolean("autoNight", d.autoNight),
            margins = enumOr(prefs.getString("margins", null), d.margins),
            spacing = enumOr(prefs.getString("spacing", null), d.spacing),
            hyphenate = prefs.getBoolean("hyphenate", d.hyphenate),
            volumeKeys = prefs.getBoolean("volumeKeys", d.volumeKeys),
            keepScreenOn = prefs.getBoolean("keepScreenOn", d.keepScreenOn),
            seenTapGuide = prefs.getBoolean("seenTapGuide", d.seenTapGuide),
        )
    }

    fun update(f: (ReaderSettings) -> ReaderSettings) {
        val s = f(_settings.value)
        _settings.value = s
        prefs.edit()
            .putInt("fontSize", s.fontSizeSp).putString("typeface", s.typeface.name).putString("theme", s.theme.name)
            .putBoolean("autoNight", s.autoNight).putString("margins", s.margins.name).putString("spacing", s.spacing.name)
            .putBoolean("hyphenate", s.hyphenate).putBoolean("volumeKeys", s.volumeKeys)
            .putBoolean("keepScreenOn", s.keepScreenOn).putBoolean("seenTapGuide", s.seenTapGuide)
            .apply()
    }

    private inline fun <reified T : Enum<T>> enumOr(name: String?, default: T): T =
        enumValues<T>().firstOrNull { it.name == name } ?: default
}

/**
 * The phone's own fonts, handed to the engine: Noto Serif for book text when chosen, and the Noto
 * families for scripts the text face lacks (Devanagari and other Indic scripts, CJK, Arabic, ...).
 */
object SystemFonts {
    private const val DIR = "/system/fonts"

    private fun find(vararg names: String): String? =
        names.map { File(DIR, it) }.firstOrNull { it.canRead() }?.path

    fun serif(): Triple<String?, String?, String?> = Triple(
        find("NotoSerif-Regular.ttf"),
        find("NotoSerif-Bold.ttf"),
        find("NotoSerif-Italic.ttf"),
    )

    /** Fallback fonts in priority order; static (non-variable) faces where the phone has them. */
    val fallbacks: List<String> by lazy {
        val files = File(DIR).listFiles()?.map { it.name }?.toSet() ?: emptySet()
        fun pick(family: String): String? =
            listOf("$family-Regular.ttf", "$family-Regular.otf", "$family-VF.ttf", "$family.ttf")
                .firstOrNull { it in files }?.let { "$DIR/$it" }
        val scripts = listOf(
            "NotoSansDevanagari", "NotoSerifDevanagari", "NotoSansBengali", "NotoSansGujarati", "NotoSansGurmukhi",
            "NotoSansTamil", "NotoSansTelugu", "NotoSansKannada", "NotoSansMalayalam", "NotoSansOriya",
            "NotoSansSinhala", "NotoNaskhArabic", "NotoSansHebrew", "NotoSansThai", "NotoSansArmenian",
            "NotoSansGeorgian", "NotoSansEthiopic", "NotoSansSymbols-Regular-Subsetted", "NotoSansSymbols2",
        )
        val list = scripts.mapNotNull { pick(it) }.toMutableList()
        // CJK last: it is large, and only needed for Chinese, Japanese and Korean text
        listOf("NotoSansCJK-Regular.ttc", "NotoSansSC-Regular.otf", "DroidSansFallback.ttf", "DroidSansFallbackFull.ttf")
            .firstOrNull { it in files }?.let { list.add("$DIR/$it") }
        list
    }
}

/**
 * The font data handed to the engine (which bundles none): the app's own DejaVu (Sans), the phone's
 * Noto Serif when it has it (Serif), IBM Plex Mono for code, and the phone's fonts for other scripts.
 */
object FontBank {
    private lateinit var app: Context
    private val cache = HashMap<String, ByteArray>()

    fun init(context: Context) {
        app = context.applicationContext
    }

    @Synchronized
    private fun asset(name: String): ByteArray = cache.getOrPut("a:$name") { app.assets.open("fonts/$name").use { it.readBytes() } }

    @Synchronized
    private fun file(path: String?): ByteArray? = path?.let { p -> cache.getOrPut("f:$p") { File(p).readBytes() } }

    @Synchronized
    private fun mono(): ByteArray = cache.getOrPut("mono") { app.resources.openRawResource(R.font.plex_mono_regular).use { it.readBytes() } }

    fun files(t: Typeface): uniffi.engine_ffi.FontFiles {
        val sans = Triple(asset("DejaVuSans.ttf"), asset("DejaVuSans-Bold.ttf"), asset("DejaVuSans-Oblique.ttf"))
        val (r, b, i) = when (t) {
            Typeface.Serif -> SystemFonts.serif().let { (r, b, i) ->
                Triple(file(r) ?: sans.first, file(b) ?: sans.second, file(i) ?: sans.third)
            }
            Typeface.Sans -> sans
        }
        return uniffi.engine_ffi.FontFiles(r, b, i, mono(), SystemFonts.fallbacks)
    }
}

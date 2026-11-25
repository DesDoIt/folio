package io.folio.reader

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** App settings: the app theme (with previews) and light or dark. */
@Composable
fun SettingsScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val store = remember { AppearanceStore.get(context) }
    val current by store.theme.collectAsState()
    val mode by store.mode.collectAsState()
    val l = look
    val systemDark = isSystemInDarkTheme()
    val dark = when (mode) { DarkMode.System -> systemDark; DarkMode.Light -> false; DarkMode.Dark -> true }
    androidx.activity.compose.BackHandler(onBack = onBack)
    Scaffold(containerColor = l.p.paper, topBar = { Header(listOf("Settings"), onBack = onBack) }) { pad ->
        Column(Modifier.padding(pad).fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 32.dp)) {
            SectionLabel("App theme")
            Text("How Folio looks around your books. The page colours are set in the reader.",
                 style = Type.body, color = l.p.muted, modifier = Modifier.padding(horizontal = 16.dp))
            Spacer(Modifier.height(12.dp))
            Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Themes.all.forEach { t ->
                    ThemeCard(t, selected = t.id == current.id, dark = dark) { store.setTheme(t) }
                }
            }
            Spacer(Modifier.height(20.dp))
            SectionLabel("Light or dark")
            Box(Modifier.padding(horizontal = 16.dp)) {
                Segmented(DarkMode.entries, mode, { it.label }) { store.setMode(it) }
            }
            Spacer(Modifier.height(24.dp))
            SectionLabel("Book sources")
            BookSourcesSettings()
            Spacer(Modifier.height(8.dp))
            SectionLabel("About")
            Text("Folio ${BuildConfigCompat.versionName(context)} · Open source (GPL-3.0). Interface type: IBM Plex Mono (SIL OFL).",
                 style = Type.meta, color = l.p.muted, modifier = Modifier.padding(horizontal = 16.dp))
        }
    }
}

/** A theme, drawn in itself: a miniature of its header, buttons and progress. */
@Composable
private fun ThemeCard(t: AppTheme, selected: Boolean, dark: Boolean, onPick: () -> Unit) {
    val outer = look
    val unlocked = Entitlements.unlocked(t)
    val preview = Look(t, if (dark) t.dark else t.light)
    Column(
        Modifier.fillMaxWidth()
            .border(BorderStroke(if (selected) 3.dp else 1.dp, if (selected) outer.p.accent else outer.p.faint), outer.smallShape)
            .clickable(enabled = unlocked, onClick = onPick).padding(3.dp),
    ) {
        CompositionLocalProvider(LocalLook provides preview) {
            val p = preview.p
            Column(Modifier.fillMaxWidth().background(p.paper).padding(14.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(if (t.pathHeader) "LIBRARY" else "Library", style = if (t.pathHeader) Type.heading else Type.display.copy(fontSize = 22.sp, lineHeight = 26.sp),
                         color = p.ink, modifier = Modifier.weight(1f))
                    if (selected) Tag("In use", inverted = true)
                }
                Spacer(Modifier.height(6.dp))
                if (t.id == "eink") Rule(thickness = 2.dp) else if (t.id == "editorial") Rule(thickness = 1.5.dp)
                Spacer(Modifier.height(10.dp))
                Text("The Count of Monte Cristo", style = Type.title, color = p.ink)
                Text(preview.caps("Alexandre Dumas"), style = Type.label, color = p.muted)
                Spacer(Modifier.height(8.dp))
                ProgressMeter(0.42f, Modifier.fillMaxWidth())
                Spacer(Modifier.height(12.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    InkButton("Read", {}, filled = true, compact = true)
                    InkButton("Get", {}, compact = true)
                }
            }
        }
        Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(outer.caps(t.name), style = Type.button.copy(fontWeight = FontWeight.Bold), color = outer.p.ink)
                Text(t.blurb, style = Type.meta, color = outer.p.muted)
            }
            if (!unlocked) {
                Spacer(Modifier.width(8.dp))
                Tag("Supporter")
            }
        }
    }
}

/** The app's version name, without generating BuildConfig. */
object BuildConfigCompat {
    fun versionName(context: android.content.Context): String = try {
        context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: ""
    } catch (e: Exception) {
        ""
    }
}

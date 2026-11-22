# Kindle Linux

A small, self-contained Linux environment for old jailbroken e-ink Kindles: a slim
**Alpine Linux** system with a touch GUI written in **Rust**, drawn straight to the e-ink
framebuffer (no X11). It turns a Kindle into an independent little Linux device that can
still read books via KOReader.

Developed on a **Kindle Basic 7th gen (2014, "KT2", Wario platform, firmware 5.12.2.2)**:
i.MX6 SoloLite, 256 MB RAM, 600×800 8-bit e-ink, zForce2 touchscreen, ath6kl Wi-Fi.
Other Wario-era Kindles (Paperwhite 2/3, Voyage) are likely close, but untested.

> ⚠️ Experimental. Use at your own risk. Back up your Kindle's storage first.

## Screenshots

Rendered by the built-in simulator (`tools/screenshots.sh`), pixel-for-pixel what the 600×800 e-ink panel shows.

| | | |
|:-:|:-:|:-:|
| <img src="docs/screenshots/home.png" width="240" alt="Home screen"><br>Home | <img src="docs/screenshots/reader.png" width="240" alt="Built-in reader"><br>Reader | <img src="docs/screenshots/word-menu.png" width="240" alt="Long-press a word"><br>Define / highlight |
| <img src="docs/screenshots/get-books.png" width="240" alt="Get Books"><br>Get Books | <img src="docs/screenshots/books.png" width="240" alt="Library"><br>Library | <img src="docs/screenshots/send-to-kindle.png" width="240" alt="Send to Kindle"><br>Send to Kindle |
| <img src="docs/screenshots/news.png" width="240" alt="News feed"><br>News | <img src="docs/screenshots/news-article.png" width="240" alt="News article"><br>Article | <img src="docs/screenshots/wikipedia.png" width="240" alt="Wikipedia article"><br>Wikipedia |
| <img src="docs/screenshots/web-start.png" width="240" alt="Web browser"><br>Web | <img src="docs/screenshots/web-results.png" width="240" alt="Search results"><br>Search results | <img src="docs/screenshots/sleep-book.png" width="240" alt="Sleep screen: current book"><br>Sleep: current book |
| <img src="docs/screenshots/sleep-weather.png" width="240" alt="Sleep screen: weather"><br>Sleep: weather | <img src="docs/screenshots/sleep-highlight.png" width="240" alt="Sleep screen: highlight"><br>Sleep: highlight | <img src="docs/screenshots/sleep-date.png" width="240" alt="Sleep screen: date"><br>Sleep: date |
| <img src="docs/screenshots/chess.png" width="240" alt="Chess"><br>Chess | <img src="docs/screenshots/sudoku.png" width="240" alt="Sudoku"><br>Sudoku | <img src="docs/screenshots/sketch.png" width="240" alt="Sketchpad"><br>Sketch |
| <img src="docs/screenshots/highlights.png" width="240" alt="Highlights"><br>Highlights | <img src="docs/screenshots/stats.png" width="240" alt="Reading stats"><br>Reading stats | <img src="docs/screenshots/terminal.png" width="240" alt="Terminal"><br>Terminal |
| <img src="docs/screenshots/files.png" width="240" alt="File manager"><br>Files | <img src="docs/screenshots/settings.png" width="240" alt="Settings"><br>Settings | <img src="docs/screenshots/boot-menu.png" width="240" alt="Boot menu"><br>Boot menu |

## Features

**Reading**
- **Reader**: built-in, for EPUB, TXT and saved web pages: full-screen pages, tap or swipe to turn, chapters,
  font sizes, position saved per book. Long-press a word to **define** it (English, online) or **highlight**
  its sentence
- **Books**: your library; tap opens the built-in reader (EPUB, TXT, saved pages) or, for other formats
  (PDF, MOBI/AZW3, comics), KOReader; hold any book to open it in KOReader
- **Get Books**: search/browse Project Gutenberg and Standard Ebooks, or download any EPUB link
- **Send to Kindle**: open `http://<kindle-ip>:8080` on a phone or computer and upload books over Wi-Fi
- **Highlights**: everything you highlighted, plus import of the Kindle's `My Clippings.txt`
- **Reading stats**: minutes and pages per day, streaks, books finished, a 14-day chart

**Internet**
- **News**: RSS/Atom feeds; articles open in the reader, full text fetched when feeds only carry summaries,
  "Save for later" for offline reading
- **Wikipedia**: search or random article, sections become chapters, every opened article cached offline;
  8 languages
- **Web**: touch browser in reading mode (text + tappable links, no JavaScript), choice of search engine,
  bookmarks, save-for-offline, EPUB downloads, "Open in Kindle's browser" for full pages
- **Sleep screens**: your current book (cover + progress), a random highlight, your own pictures (PNG/JPG in
  `screensavers/`), weather (current conditions and a 5-day forecast from Open-Meteo, no account) or a clean
  date screen. Sleeps on the power button, from the Power menu, or after 2–30 minutes of inactivity

**Play & make**
- **Games**: Sudoku (3 levels, unique-solution puzzles) and Chess against a small built-in engine
- **Sketch**: finger drawing with fast e-ink updates, saved as PNG (viewable from Files)
- **Notes** and a simple **text editor**

**System**
- **Terminal** with a built-in VT100 emulator and on-screen keyboard (Esc, Tab, Ctrl, arrows)
- **Files**: browse, rename, delete, create files and folders
- **Wi-Fi** via its own `wpa_supplicant`, **SSH**/SFTP via dropbear
- **Settings**, **System** info, **Power** (sleep, exit to stock Kindle, reboot, power off)
- **Boot menu**: optionally boot straight into Kindle Linux, with automatic fallback to the stock Kindle
- Text in other scripts (Chinese, Japanese, Korean, Hebrew…) via fallback fonts; Devanagari and Arabic show
  their letters but not their joined forms (no shaping engine yet)

## How it works

```
Kindle kernel 3.0.35 (Amazon's, unchanged)
└── host/kos-launch.sh        mounts the image, stops Amazon's UI, wifid, volumd
    └── chroot: Alpine 3.24 armv7 (out/kos-rootfs.ext3, ~27 MB used)
        └── /opt/kos/kos      Rust GUI, static binary (~2.6 MB, fonts embedded)
```

Amazon's `powerd` is kept for now, because it owns safe suspend/resume on this kernel.
Everything else above the kernel is ours. On exit, the launcher always unmounts the image
and restarts the stock Kindle services.

| Code | Meaning (GUI exit → launcher) |
|---|---|
| 0 | exit to stock Kindle |
| 3 | restart GUI |
| 10 | open KOReader (path in `/mnt/us/kos/request`) |
| 20 / 21 | reboot / power off |

## Repository layout

| Path | What |
|---|---|
| `src/` | Rust GUI: `fb.rs` (EPDC framebuffer), `input.rs` (touch), `canvas.rs` (drawing + fonts), `ui.rs` (toolkit), `apps.rs` (home, books, files, notes), `reader.rs` + `epub.rs` (reader), `getbooks.rs`, `upload.rs`, `news.rs`, `wiki.rs`, `web.rs`, `highlights.rs` (highlights, dictionary, stats), `games.rs`, `sketch.rs`, `dashboard.rs`, `term.rs`, `settings.rs`, `sys.rs` |
| `host/` | Kindle-side launcher and library scriptlet |
| `rootfs/` | Files overlaid onto the Alpine image |
| `build/` | Docker-based build scripts (no local Rust or ARM emulation needed) |
| `tools/` | On-device diagnostics scriptlet, sample EPUB generator, screenshot script |
| `docs/screenshots/` | README screenshots (regenerate with `tools/screenshots.sh`) |
| `tests/fixtures/` | `sample.epub` used by the simulator |
| `fonts/` | DejaVu fonts (embedded into the binary) |

## Building

Needs Docker (or Podman) on an arm64 or x86_64 host.

```sh
build/build.sh            # cross-compiles the GUI, builds out/kos-rootfs.ext3
build/cargo.sh sim        # runs the GUI simulator, writes PNG screenshots to sim-out/
build/cargo.sh test       # unit tests
build/install.sh          # copies everything to a Kindle mounted at /Volumes/Kindle
tools/screenshots.sh      # regenerates docs/screenshots/
```

## Installing

Requires a jailbroken Kindle (see kindlemodding.org). KOReader (installed with KPM) is optional: Kindle
Linux uses it for PDFs, MOBI/AZW3 and comics. To keep it out of the stock Kindle's library, delete
`documents/KOReader.sh`; Kindle Linux launches KOReader directly from `koreader/`.

1. Connect the Kindle over USB and run `build/install.sh` (or copy by hand: `out/kos-rootfs.ext3`,
   `out/kos` and `host/kos-*.sh`, `host/kindle-linux.conf` to `kos/`, and `host/KindleLinux*.sh` to
   `documents/`).
2. Eject, then tap **Kindle Linux** in the library.
3. Optional: tap **Kindle Linux: boot ON** to get a boot menu at power-on (5 s countdown, defaults to
   Kindle Linux); **boot OFF** removes it.

### Escape hatches

- Create `kos/disable` on the USB drive to stop it from starting (also at boot).
- If Kindle Linux hangs before drawing its first screen, the next boot goes to the stock Kindle automatically.
- Hold the power button about 15 seconds to force a reboot into the stock Kindle.
- `kos/kos.log` on the USB drive has the launcher and GUI logs.
- Create `kos/debug-touch` to log raw touch coordinates to `kos/touch.log`.

## Tools

`tools/KOS-Diagnostics.sh` is a read-only scriptlet that dumps hardware details
(framebuffer, input devices, modules, services, battery) to `kos-diag.txt`. It helps
when porting to another model. The output contains your device's serial number, so
don't publish it.

## License

Kindle Linux is free software under the **GNU General Public License v3.0 or later**; see `LICENSE`.

Third-party components (Rust crates, DejaVu and Droid fonts, Alpine packages in the built image) and
data sources (Open-Meteo weather under CC BY 4.0, Wikipedia text under CC BY-SA 4.0) are listed with
their licenses in [`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md).

Not affiliated with or endorsed by Amazon. "Kindle" is a trademark of Amazon.com, Inc. This project
does not remove DRM; Amazon store books remain readable only in the stock Kindle software.

# Third-party licenses

Folio itself is licensed under the **GNU General Public License v3.0 or later** (see `LICENSE`).
It builds on the following third-party work. All of it is compatible with GPL-3.0-or-later.

## Rust crates compiled into the `kos` binary

| Crate | Version | License |
|---|---|---|
| ab_glyph | 0.2.32 | Apache-2.0 |
| ab_glyph_rasterizer | 0.1.10 | Apache-2.0 |
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| allocator-api2 | 0.2.21 | MIT OR Apache-2.0 |
| arrayvec | 0.7.8 | MIT OR Apache-2.0 |
| bitflags | 1.3.2 | MIT OR Apache-2.0 |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 |
| core_maths | 0.1.1 | MIT |
| crc32fast | 1.5.5 | MIT OR Apache-2.0 |
| equivalent | 1.0.3 | Apache-2.0 OR MIT |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 |
| flate2 | 1.1.11 | MIT OR Apache-2.0 |
| foldhash | 0.1.5 | Zlib |
| fontdue | 0.9.5 | MIT OR Apache-2.0 OR Zlib |
| hashbrown | 0.15.5 | MIT OR Apache-2.0 |
| libc | 0.2.191 | MIT OR Apache-2.0 |
| libm | 0.2.16 | MIT |
| log | 0.4.35 | MIT OR Apache-2.0 |
| miniz_oxide | 0.8.9 / 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| owned_ttf_parser | 0.25.1 | Apache-2.0 |
| png | 0.17.16 | MIT OR Apache-2.0 |
| rustybuzz | 0.20.1 | MIT |
| simd-adler32 | 0.3.10 | MIT |
| smallvec | 1.16.6 | MIT OR Apache-2.0 |
| ttf-parser | 0.25.1 | MIT OR Apache-2.0 |
| unicode-bidi-mirroring | 0.4.0 | MIT OR Apache-2.0 |
| unicode-ccc | 0.4.0 | MIT OR Apache-2.0 |
| unicode-properties | 0.1.5 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| unicode-script | 0.5.9 | MIT OR Apache-2.0 |
| utf8parse | 0.2.2 | Apache-2.0 OR MIT |
| vte | 0.13.1 | Apache-2.0 OR MIT |
| zlib-rs | 0.6.8 | Zlib |
| zune-core | 0.4.12 | MIT OR Apache-2.0 OR Zlib |
| zune-jpeg | 0.4.21 | MIT OR Apache-2.0 OR Zlib |

Build-time only (procedural macros, not shipped in the binary): `proc-macro2`, `quote`
(MIT OR Apache-2.0), `unicode-ident` ((MIT OR Apache-2.0) AND Unicode-3.0),
`vte_generate_state_changes` (Apache-2.0 OR MIT).

The list is generated from `cargo metadata`; regenerate it when dependencies change.
When distributing a compiled binary, include this file and the licenses it refers to.

## Fonts

- **DejaVu Sans, DejaVu Sans Bold, DejaVu Sans Mono** (`platforms/kindle/fonts/`, embedded in the Folio OS binary;
  Sans, Bold and Oblique also in `platforms/android/app/src/main/assets/fonts/`, the Android app's Sans reading face): Bitstream Vera
  Fonts license, with DejaVu changes in the public domain and Arev glyphs under their own permissive terms.
  Full text: `platforms/kindle/fonts/DejaVu-LICENSE` (also in the Android app's `assets/licenses/`).
- **DejaVu Sans Oblique** (same places): same license as the other DejaVu fonts.
- **Droid Sans Fallback and other Droid non-Latin fonts** (installed in the Alpine image from the
  `font-droid-nonlatin` package, not stored in this repository): Apache-2.0.
- **IBM Plex Mono** Regular, Medium, SemiBold, Bold (`platforms/android/app/src/main/res/font/`, the Android app's
  interface type): SIL Open Font License 1.1, © IBM Corp. Full text:
  `platforms/android/app/src/main/assets/licenses/IBMPlexMono-OFL.txt`.

## Hyphenation patterns

`data/hyph-en-us.pat.txt`: US English hyphenation patterns from the hyph-utf8 project
(https://github.com/hyphenation/tex-hyphen), Copyright (C) 1990, 2004, 2005 Gerard D.C. Kuiken.
Copying and distribution, with or without modification, are permitted in any medium without
royalty provided the copyright notice and this notice are preserved (`data/hyph-en-us.LICENSE`).

## Alpine Linux image

`platforms/kindle/build/build.sh` downloads packages from the Alpine Linux v3.24 repositories (BusyBox, musl, curl,
wpa_supplicant, iw, dropbear, nano, htop, MuPDF tools, DjVuLibre, libarchive tools and their dependencies)
to assemble `out/kos-rootfs.sqsh`. The page reader runs three of these as separate programs:
**MuPDF** `mutool` (AGPL-3.0-or-later) for PDF, **DjVuLibre** `ddjvu`/`djvused` (GPL-2.0-or-later) for
DjVu, and **libarchive** `bsdtar` (BSD-2-Clause) for CBR comics.
The image is **not** part of this repository. Those packages keep their own licenses (GPL-2.0, MIT, BSD
and others; listed per package at https://pkgs.alpinelinux.org). If you distribute a built image, you
must comply with those licenses, including offering the corresponding source for GPL packages, which
Alpine publishes at https://gitlab.alpinelinux.org/alpine/aports.

## Online services used at runtime

Folio fetches data from these services only when you use the related feature:

- **Open-Meteo** (weather): data under CC BY 4.0; attributed on the weather screen.
- **Wikipedia**: article text under CC BY-SA 4.0.
- **Project Gutenberg** (via the Gutendex API and gutenberg.org mirrors) and **Standard Ebooks**
  (CC0 1.0 for their production work): public-domain books.
- **dictionaryapi.dev** (definitions), **DuckDuckGo**, **Marginalia** (search).
- **KOReader-compatible sync server** (only if you sign in): a book fingerprint and your reading position.
- **Wikisource** (search through the MediaWiki API, EPUBs from the Wikisource export service
  ws-export.wmcloud.org): public-domain works; transcriptions and Wikisource's additions under
  CC BY-SA 4.0. The exported EPUB carries its own credits page.
- **Project Gutenberg Australia**, **Faded Page** (Canada) and the **Internet Archive** (Android; each
  can be turned off): books in the public domain in Australia or Canada, or in the US by publication date or
  as marked by the uploader. Their terms may differ from your country's; Folio says so where you turn
  them on.
- **OPDS catalogs** you add yourself (e.g. your Calibre server).

## Screenshots (`platforms/kindle/screenshots/`, `platforms/android/screenshots/`)

Rendered by the simulator. They show public-domain book titles from Project Gutenberg and Standard
Ebooks (including the Standard Ebooks cover of "The Time Machine", CC0), weather data from Open-Meteo (CC BY 4.0), and Wikipedia text (CC BY-SA 4.0, from the articles
"Tokyo" and "Electronic paper"). News screenshots use the original sample feed in
`tests/fixtures/sample-feed.xml`; the sample book is original filler text generated by
`tools/make-sample-epub.py`.

The Android screenshots were taken in the Android emulator. They show Standard Ebooks covers (CC0),
Project Gutenberg titles, news from NASA's public feed (NASA material is not protected by copyright
in the United States), and the same sample book.

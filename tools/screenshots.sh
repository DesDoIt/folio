#!/bin/sh
# Regenerate the README screenshots from the simulator (needs network for Get Books/Web/weather).
set -e
cd "$(dirname "$0")/.."
rm -rf sim-out
build/cargo.sh sim sim-out >/dev/null 2>&1 || true
mkdir -p docs/screenshots
for pair in home-weather:home books:books reader-page1:reader reader-menu:reader-menu reader-chapters:chapters \
            getbooks-gutenberg:get-books getbooks-downloaded:get-books-downloaded web-start:web-start \
            web-wikipedia-0:web-results dashboard-sleep:dashboard terminal:terminal editor-typed:editor \
            wifi-password:wifi settings-dashboard:settings files-docs:files power:power boot-menu:boot-menu \
            news-items:news news-article:news-article wiki-article:wikipedia wiki-sections:wikipedia-sections \
            reader-word-menu:word-menu highlights:highlights stats:stats chess:chess sudoku:sudoku sketch:sketch \
            send-to-kindle:send-to-kindle; do
    src=${pair%%:*}; dst=${pair##*:}
    f=$(ls sim-out/*-"$src".png 2>/dev/null | tail -1)
    if [ -n "$f" ]; then cp "$f" "docs/screenshots/$dst.png"; else echo "missing: $src"; fi
done
ls docs/screenshots

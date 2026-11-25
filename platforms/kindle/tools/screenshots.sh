#!/bin/sh
# Regenerate the README screenshots from the simulator (needs network for Get Books/Web/weather).
set -e
cd "$(dirname "$0")/../../.."
rm -rf sim-out
platforms/kindle/build/cargo.sh sim sim-out >/dev/null 2>&1 || true
mkdir -p platforms/kindle/screenshots
for pair in home-weather:home books-covers:books books-list:books-list pdf-page:pdf pdf-fit-width:pdf-fit-width comic-page:comic getbooks-progress:get-books-progress news-headlines-topic:news-headlines news-feeds:news-feeds news-browse:news-browse reader-page1:reader reader-toolbar:reader-toolbar reader-menu:reader-menu reader-chapters:chapters \
            getbooks-gutenberg:get-books getbooks-downloaded:get-books-downloaded web-start:web-start \
            web-wikipedia-0:web-results sleep-book-cover:sleep-book sleep-weather:sleep-weather sleep-highlight:sleep-highlight sleep-date:sleep-date terminal:terminal editor-typed:editor \
            wifi-password:wifi settings-dashboard:settings files-docs:files power:power boot-menu:boot-menu \
            news-items:news news-article:news-article wiki-article:wikipedia wiki-sections:wikipedia-sections \
            reader-word-menu:word-menu highlights:highlights stats:stats chess:chess sudoku:sudoku sketch:sketch \
            send-to-kindle:send-to-kindle setup-welcome:setup setup-sleep:setup-sleep settings-large:settings-large; do
    src=${pair%%:*}; dst=${pair##*:}
    f=$(ls sim-out/*-"$src".png 2>/dev/null | tail -1)
    if [ -n "$f" ]; then cp "$f" "platforms/kindle/screenshots/$dst.png"; else echo "missing: $src"; fi
done
ls platforms/kindle/screenshots

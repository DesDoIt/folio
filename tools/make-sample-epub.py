#!/usr/bin/env python3
"""Generate tests/fixtures/sample.epub: a small EPUB3 exercising the reader (headings, bold,
entities, lists, quotes, rules, long words, multi-page chapters, nav TOC). Text is original filler."""
import random, zipfile, os, struct, zlib

def png_gray(w, h, pixel):
    """Minimal grayscale PNG writer (standard library only)."""
    raw = b"".join(b"\x00" + bytes(pixel(x, y) for x in range(w)) for y in range(h))
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 0, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))

random.seed(7)
WORDS = ("the river ran slowly past the old mill while lanterns swayed above quiet boats and a small "
         "dog watched the evening settle over fields of barley people talked about rain harvests trains "
         "letters friends and the long road north that nobody had walked in years").split()

def para(n):
    s = " ".join(random.choice(WORDS) for _ in range(n))
    return s[0].upper() + s[1:] + "."

def chapter(i, title, extra=""):
    ps = "\n".join(f"<p>{para(random.randint(40, 90))}</p>" for _ in range(9))
    return f"""<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>{title}</title><style>p{{margin:0}}</style></head>
<body><h1>{title}</h1>
<p>It was a <b>bright, cold</b> morning &mdash; the kind that makes &ldquo;tea&rdquo; feel necessary &amp; right.<a epub:type="noteref" href="notes.xhtml#n{i+1}"><sup>{i+1}</sup></a> The <i>Evening Chronicle</i> called it <em>extraordinarily</em> uncharacteristic, notwithstanding the meteorological forecasts.</p>
{extra}
{ps}
</body></html>"""

extras = [
    "<p><img src=\"../images/mill.png\" alt=\"The old mill\"/></p><blockquote><p>A quoted passage sits indented from the rest of the text, as quotes do.</p></blockquote>",
    "<ul><li>First item on a short list</li><li>Second item, a little longer than the first one</li><li>Third</li></ul><hr/>",
    "<p>Supercalifragilisticexpialidociousandthensomemorelettersthatkeepgoing tests word breaking.</p><h2>A smaller heading</h2>",
]
titles = ["Chapter One: The Mill", "Chapter Two: The Road North", "Chapter Three: Rain"]

out = os.path.join(os.path.dirname(__file__), "..", "tests", "fixtures", "sample.epub")
with zipfile.ZipFile(out, "w") as z:
    z.writestr(zipfile.ZipInfo("mimetype"), "application/epub+zip", compress_type=zipfile.ZIP_STORED)
    z.writestr("META-INF/container.xml", """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>""")
    items = "\n".join(f'<item id="c{i}" href="text/ch{i}.xhtml" media-type="application/xhtml+xml"/>' for i in range(3))
    items += '\n<item id="notes" href="text/notes.xhtml" media-type="application/xhtml+xml"/>'
    items += '\n<item id="img" href="images/mill.png" media-type="image/png"/>'
    spine = "\n".join(f'<itemref idref="c{i}"/>' for i in range(3)) + '\n<itemref idref="notes" linear="yes"/>'
    # a simple drawing: sky gradient, ground, a mill silhouette
    def mill(x, y):
        if y > 150:
            return 90
        if 120 < x < 180 and 60 < y <= 150:
            return 30
        if abs((x - 150) - (y - 60)) < 6 or abs((x - 150) + (y - 60)) < 6:
            return 30 if 20 < y < 100 else 200 + y // 8
        return 200 + y // 8
    z.writestr("OEBPS/images/mill.png", png_gray(300, 200, mill))
    notes = "".join(f'<p id="n{k}">{k}. A short note for chapter {k}: the morning was in fact quite ordinary.</p>' for k in (1, 2, 3))
    z.writestr("OEBPS/text/notes.xhtml", f'''<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Notes</title></head><body><h1>Notes</h1>{notes}</body></html>''')
    z.writestr("OEBPS/content.opf", f"""<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">sample</dc:identifier>
<dc:title>A Sample Book</dc:title><dc:creator>Sample Author</dc:creator><dc:language>en</dc:language></metadata>
<manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
{items}</manifest><spine>{spine}</spine></package>""")
    links = "\n".join(f'<li><a href="text/ch{i}.xhtml">{t}</a></li>' for i, t in enumerate(titles))
    z.writestr("OEBPS/nav.xhtml", f"""<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Contents</title></head>
<body><nav epub:type="toc"><ol>{links}</ol></nav></body></html>""")
    for i, t in enumerate(titles):
        z.writestr(f"OEBPS/text/ch{i}.xhtml", chapter(i, t, extras[i]), compress_type=zipfile.ZIP_DEFLATED)
print("wrote", os.path.normpath(out), os.path.getsize(out), "bytes")

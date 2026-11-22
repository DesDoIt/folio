#!/usr/bin/env python3
"""Write tests/fixtures/sample.pdf: a small A5 PDF with original filler text, standard (non-embedded)
fonts and bookmarks, for testing the page reader. Also tests/fixtures/sample.cbz from the sample pictures."""
import os, random, zipfile

random.seed(7)
WORDS = "river lantern harbour morning garden village bridge train orchestra museum bakery harvest island letter".split()

def sentence():
    w = [random.choice(WORDS) for _ in range(random.randint(8, 14))]
    return (" ".join(w)).capitalize() + "."

chapters = ["The Harbour", "The Night Train", "The Bakery", "The Orchestra"]
pages = []
for c, title in enumerate(chapters):
    for part in range(2):
        lines = []
        y = 560
        if part == 0:
            lines.append(("F2", 18, 50, y, f"Chapter {c + 1}: {title}"))
            y -= 40
        text = " ".join(sentence() for _ in range(26))
        line = ""
        for word in text.split():
            if len(line) + len(word) > 62:
                lines.append(("F1", 10, 50, y, line))
                y -= 14
                line = ""
            line += word + " "
            if y < 60:
                break
        if line and y >= 60:
            lines.append(("F1", 10, 50, y, line))
        lines.append(("F1", 8, 200, 30, f"- {len(pages) + 1} -"))
        pages.append(lines)

objs = []
def add(body):
    objs.append(body)
    return len(objs)

catalog = add(None)
pages_obj = add(None)
f1 = add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>")
f2 = add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>")
page_ids = []
for lines in pages:
    ops = []
    for font, size, x, y, t in lines:
        t = t.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
        ops.append(f"BT /{font} {size} Tf {x} {y} Td ({t}) Tj ET")
    stream = "\n".join(ops).encode()
    content = add(b"<< /Length %d >>\nstream\n" % len(stream) + stream + b"\nendstream")
    page_ids.append(add(None))
    objs[page_ids[-1] - 1] = (f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 420 595] "
                              f"/Resources << /Font << /F1 {f1} 0 R /F2 {f2} 0 R >> >> /Contents {content} 0 R >>").encode()
objs[pages_obj - 1] = (f"<< /Type /Pages /Count {len(page_ids)} /Kids [{' '.join(f'{p} 0 R' for p in page_ids)}] >>").encode()
# outline: one bookmark per chapter
outline = add(None)
items = [add(None) for _ in chapters]
for i, (it, title) in enumerate(zip(items, chapters)):
    prev = f"/Prev {items[i - 1]} 0 R " if i > 0 else ""
    nxt = f"/Next {items[i + 1]} 0 R " if i + 1 < len(items) else ""
    objs[it - 1] = (f"<< /Title (Chapter {i + 1}: {title}) /Parent {outline} 0 R {prev}{nxt}"
                    f"/Dest [{page_ids[i * 2]} 0 R /Fit] >>").encode()
objs[outline - 1] = f"<< /Type /Outlines /First {items[0]} 0 R /Last {items[-1]} 0 R /Count {len(items)} >>".encode()
objs[catalog - 1] = f"<< /Type /Catalog /Pages {pages_obj} 0 R /Outlines {outline} 0 R /PageMode /UseOutlines >>".encode()

out = bytearray(b"%PDF-1.4\n")
offsets = []
for i, body in enumerate(objs, 1):
    offsets.append(len(out))
    out += f"{i} 0 obj\n".encode() + body + b"\nendobj\n"
xref = len(out)
out += f"xref\n0 {len(objs) + 1}\n0000000000 65535 f \n".encode()
for o in offsets:
    out += f"{o:010d} 00000 n \n".encode()
out += f"trailer\n<< /Size {len(objs) + 1} /Root {catalog} 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
here = os.path.join(os.path.dirname(__file__), "..", "tests", "fixtures")
open(os.path.join(here, "sample.pdf"), "wb").write(out)

with zipfile.ZipFile(os.path.join(here, "sample.cbz"), "w") as z:
    for k in range(1, 5):
        z.write(os.path.join(here, f"thumb-{k}.png"), f"comic/page{k}.png")

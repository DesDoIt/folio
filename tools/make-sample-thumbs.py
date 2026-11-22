#!/usr/bin/env python3
"""Generate simple original greyscale pictures for the sample news feed (no dependencies)."""
import math, struct, zlib, sys, os

W, H = 240, 180

def png(path, px):
    raw = b"".join(b"\x00" + bytes(px[y * W:(y + 1) * W]) for y in range(H))
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 0, 0, 0, 0)) \
        + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    open(path, "wb").write(data)

def scene(kind):
    px = []
    for y in range(H):
        for x in range(W):
            if kind == 0:   # sun over hills
                v = 200 + y // 4
                if (x - 170) ** 2 + (y - 55) ** 2 < 26 ** 2: v = 250
                if y > 120 + 18 * math.sin(x / 30): v = 90
                if y > 140 + 12 * math.sin(x / 18 + 1): v = 50
            elif kind == 1:  # rails to the horizon
                v = 225 - y // 3
                if y > 90:
                    t = (y - 90) / 90
                    for rx in (120 - 90 * t, 120 + 90 * t):
                        if abs(x - rx) < 1 + 3 * t: v = 30
                    if int((y - 90) ** 1.3) % 14 < 3 and abs(x - 120) < 100 * t: v = 70
            elif kind == 2:  # loaves
                v = 235
                for cx, cy in ((70, 110), (150, 100), (200, 125)):
                    d = ((x - cx) / 42) ** 2 + ((y - cy) / 28) ** 2
                    if d < 1: v = int(90 + 120 * d)
                if y > 140: v = 160
            else:            # organ pipes
                v = 220
                col = x // 20
                top = 40 + abs(col - 6) * 10
                if y > top and x % 20 < 14: v = 60 + (x % 20) * 8
                if y > 160: v = 110
            px.append(max(0, min(255, v)))
    return px

out = sys.argv[1] if len(sys.argv) > 1 else "tests/fixtures"
for k in range(4):
    png(os.path.join(out, f"thumb-{k + 1}.png"), scene(k))

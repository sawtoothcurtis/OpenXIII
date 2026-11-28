"""Makes the flamethrower example's own pictures from code alone (no outside data):

  flamethrower/textures/charred-wood.png   charred wood: black planks with cracks and a few dying
                                           embers, fading out at its edges (a mark projected on
                                           burnt scenery)

The output is deterministic: running it again gives the same bytes.
Usage, from this folder: python make-flamethrower-textures.py
"""
import math
import os
import random
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))


def png(path, width, height, pixels):
    raw = bytearray()
    for y in range(height):
        raw.append(0)
        for x in range(width):
            raw.extend(pixels(x, y))

    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xFFFFFFFF)

    header = struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)
    data = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', header) + chunk(b'IDAT', zlib.compress(bytes(raw), 9)) + chunk(b'IEND', b'')
    with open(path, 'wb') as f:
        f.write(data)


def charred_wood():
    size = 128
    rnd = random.Random(13)
    # Planks run across the picture; each has its own darkness and a few cracks along it.
    planks = [(0.08 + rnd.random() * 0.12) for _ in range(8)]
    cracks = [(rnd.random() * size, rnd.randrange(8)) for _ in range(40)]
    embers = [(rnd.random() * size, rnd.random() * size, 1.5 + rnd.random() * 2.5) for _ in range(10)]

    def pixel(x, y):
        plank = int(y / size * 8)
        v = planks[plank]
        if y % 16 in (0, 15):                       # the gap between two planks
            v = 0.02
        for cx, p in cracks:                         # short dark cracks along the grain
            if p == plank and abs(x - cx) < 9 and abs((y % 16) - 8 + math.sin(cx) * 3) < 0.8:
                v = 0.0
        r, g, b = v * 1.15, v * 0.95, v * 0.8
        for ex, ey, er in embers:                    # a few embers still glowing
            d = math.hypot(x - ex, y - ey)
            if d < er:
                k = 1 - d / er
                r, g, b = r + 0.9 * k, g + 0.35 * k, b + 0.05 * k
        # Fades out towards the edge, so that the mark has no visible border.
        e = math.hypot(x - size / 2, y - size / 2) / (size / 2)
        a = max(0.0, min(1.0, (1.0 - e) * 2.2)) * 0.92
        return (int(min(r, 1) * 255), int(min(g, 1) * 255), int(min(b, 1) * 255), int(a * 255))

    out = os.path.join(HERE, 'flamethrower', 'textures')
    os.makedirs(out, exist_ok=True)
    png(os.path.join(out, 'charred-wood.png'), size, size, pixel)


if __name__ == '__main__':
    charred_wood()

"""Makes the asset files of the example mods from code alone (no outside data):

  medkit-crate/models/crate.gltf, crate.bin, crate.png   a 1 m box with a first-aid texture

The Blaster's model, palette texture and HUD icon are made by make-blaster-model.py (Blender).

The output is deterministic: running it again gives the same bytes.
Usage, from this folder: python make-example-assets.py
"""
import json
import math
import os
import struct
import wave
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))


def png(path, width, height, pixels, alpha):
    """Writes an 8-bit RGB or RGBA PNG; pixels(x, y) returns a tuple of 3 or 4 ints."""
    raw = bytearray()
    for y in range(height):
        raw.append(0)
        for x in range(width):
            raw.extend(pixels(x, y)[:4 if alpha else 3])

    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xFFFFFFFF)

    header = struct.pack('>IIBBBBB', width, height, 8, 6 if alpha else 2, 0, 0, 0)
    data = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', header) + chunk(b'IDAT', zlib.compress(bytes(raw), 9)) + chunk(b'IEND', b'')
    with open(path, 'wb') as f:
        f.write(data)


def wav(path, rate, samples):
    with wave.open(path, 'wb') as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(b''.join(struct.pack('<h', max(-32767, min(32767, int(round(s * 32767))))) for s in samples))


def crate():
    out = os.path.join(HERE, 'medkit-crate', 'models')
    os.makedirs(out, exist_ok=True)
    # Six faces of a 1 m box standing on y = 0: (normal, four corners in order), each face with
    # its own vertices so that normals and texture coordinates stay per face.
    faces = [
        ((1, 0, 0), [(0.5, 0, -0.5), (0.5, 1, -0.5), (0.5, 1, 0.5), (0.5, 0, 0.5)]),
        ((-1, 0, 0), [(-0.5, 0, -0.5), (-0.5, 0, 0.5), (-0.5, 1, 0.5), (-0.5, 1, -0.5)]),
        ((0, 1, 0), [(-0.5, 1, -0.5), (-0.5, 1, 0.5), (0.5, 1, 0.5), (0.5, 1, -0.5)]),
        ((0, -1, 0), [(-0.5, 0, -0.5), (0.5, 0, -0.5), (0.5, 0, 0.5), (-0.5, 0, 0.5)]),
        ((0, 0, 1), [(-0.5, 0, 0.5), (0.5, 0, 0.5), (0.5, 1, 0.5), (-0.5, 1, 0.5)]),
        ((0, 0, -1), [(-0.5, 0, -0.5), (-0.5, 1, -0.5), (0.5, 1, -0.5), (0.5, 0, -0.5)]),
    ]
    uvs = [(0, 1), (1, 1), (1, 0), (0, 0)]
    positions, normals, coords, indices = [], [], [], []
    for normal, corners in faces:
        base = len(positions)
        for c, uv in zip(corners, uvs):
            positions.append(c)
            normals.append(normal)
            coords.append(uv)
        indices += [base, base + 1, base + 2, base, base + 2, base + 3]
    blob = b''.join(struct.pack('<3f', *p) for p in positions)
    blob += b''.join(struct.pack('<3f', *n) for n in normals)
    blob += b''.join(struct.pack('<2f', *t) for t in coords)
    blob += b''.join(struct.pack('<H', i) for i in indices)
    with open(os.path.join(out, 'crate.bin'), 'wb') as f:
        f.write(blob)
    n = len(positions)
    views = [(0, 12 * n), (12 * n, 12 * n), (24 * n, 8 * n), (32 * n, 2 * len(indices))]
    gltf = {
        'asset': {'version': '2.0', 'generator': 'OpenXIII example'},
        'scene': 0,
        'scenes': [{'nodes': [0]}],
        'nodes': [{'mesh': 0}],
        'meshes': [{'primitives': [{'attributes': {'POSITION': 0, 'NORMAL': 1, 'TEXCOORD_0': 2}, 'indices': 3, 'material': 0}]}],
        'materials': [{'pbrMetallicRoughness': {'baseColorTexture': {'index': 0}}}],
        'textures': [{'source': 0}],
        'images': [{'uri': 'crate.png'}],
        'buffers': [{'uri': 'crate.bin', 'byteLength': len(blob)}],
        'bufferViews': [{'buffer': 0, 'byteOffset': o, 'byteLength': l} for o, l in views],
        'accessors': [
            {'bufferView': 0, 'componentType': 5126, 'count': n, 'type': 'VEC3', 'min': [-0.5, 0, -0.5], 'max': [0.5, 1, 0.5]},
            {'bufferView': 1, 'componentType': 5126, 'count': n, 'type': 'VEC3'},
            {'bufferView': 2, 'componentType': 5126, 'count': n, 'type': 'VEC2'},
            {'bufferView': 3, 'componentType': 5123, 'count': len(indices), 'type': 'SCALAR'},
        ],
    }
    with open(os.path.join(out, 'crate.gltf'), 'w', encoding='utf-8', newline='\n') as f:
        json.dump(gltf, f, indent=1)
        f.write('\n')

    # Texture: light planks with darker gaps, a white square with a red cross in the middle.
    def pixel(x, y):
        if 20 <= x < 44 and 20 <= y < 44:
            if (28 <= x < 36) or (28 <= y < 36):
                return (200, 30, 30)
            return (235, 235, 230)
        if x < 3 or x > 60 or y < 3 or y > 60:
            return (110, 75, 40)
        shade = 160 + 12 * ((y // 8) % 2) - (25 if y % 8 == 0 else 0)
        return (shade, int(shade * 0.72), int(shade * 0.45))

    png(os.path.join(out, 'crate.png'), 64, 64, pixel, alpha=False)


def boxes_gltf(out, name, boxes):
    """glTF model made of axis-aligned boxes: (min corner, max corner, (v0, v1) texture rows)."""
    positions, normals, coords, indices = [], [], [], []
    for (x0, y0, z0), (x1, y1, z1), (v0, v1) in boxes:
        faces = [
            ((1, 0, 0), [(x1, y0, z1), (x1, y0, z0), (x1, y1, z0), (x1, y1, z1)]),
            ((-1, 0, 0), [(x0, y0, z0), (x0, y0, z1), (x0, y1, z1), (x0, y1, z0)]),
            ((0, 1, 0), [(x0, y1, z1), (x1, y1, z1), (x1, y1, z0), (x0, y1, z0)]),
            ((0, -1, 0), [(x0, y0, z0), (x1, y0, z0), (x1, y0, z1), (x0, y0, z1)]),
            ((0, 0, 1), [(x0, y0, z1), (x1, y0, z1), (x1, y1, z1), (x0, y1, z1)]),
            ((0, 0, -1), [(x1, y0, z0), (x0, y0, z0), (x0, y1, z0), (x1, y1, z0)]),
        ]
        uvs = [(0, v1), (1, v1), (1, v0), (0, v0)]
        for normal, corners in faces:
            base = len(positions)
            for c, uv in zip(corners, uvs):
                positions.append(c)
                normals.append(normal)
                coords.append(uv)
            indices += [base, base + 1, base + 2, base, base + 2, base + 3]
    blob = b''.join(struct.pack('<3f', *p) for p in positions)
    blob += b''.join(struct.pack('<3f', *n) for n in normals)
    blob += b''.join(struct.pack('<2f', *t) for t in coords)
    blob += b''.join(struct.pack('<H', i) for i in indices)
    with open(os.path.join(out, name + '.bin'), 'wb') as f:
        f.write(blob)
    n = len(positions)
    views = [(0, 12 * n), (12 * n, 12 * n), (24 * n, 8 * n), (32 * n, 2 * len(indices))]
    lo = [min(p[k] for p in positions) for k in range(3)]
    hi = [max(p[k] for p in positions) for k in range(3)]
    gltf = {
        'asset': {'version': '2.0', 'generator': 'OpenXIII example'},
        'scene': 0,
        'scenes': [{'nodes': [0]}],
        'nodes': [{'mesh': 0}],
        'meshes': [{'primitives': [{'attributes': {'POSITION': 0, 'NORMAL': 1, 'TEXCOORD_0': 2}, 'indices': 3, 'material': 0}]}],
        'materials': [{'pbrMetallicRoughness': {'baseColorTexture': {'index': 0}}}],
        'textures': [{'source': 0}],
        'images': [{'uri': name + '.png'}],
        'buffers': [{'uri': name + '.bin', 'byteLength': len(blob)}],
        'bufferViews': [{'buffer': 0, 'byteOffset': o, 'byteLength': l} for o, l in views],
        'accessors': [
            {'bufferView': 0, 'componentType': 5126, 'count': n, 'type': 'VEC3', 'min': lo, 'max': hi},
            {'bufferView': 1, 'componentType': 5126, 'count': n, 'type': 'VEC3'},
            {'bufferView': 2, 'componentType': 5126, 'count': n, 'type': 'VEC2'},
            {'bufferView': 3, 'componentType': 5123, 'count': len(indices), 'type': 'SCALAR'},
        ],
    }
    with open(os.path.join(out, name + '.gltf'), 'w', encoding='utf-8', newline='\n') as f:
        json.dump(gltf, f, indent=1)
        f.write('\n')


if __name__ == '__main__':
    crate()
    print('example mod assets written')

# Builds a second, military look for the flamethrower example with Blender, without opening a
# window:
#
#   blender -b --python make-flamethrower-model-v2.py -- <output folder>
#
# It writes into <output folder>/models: flamethrower-lance.gltf (the gun held in the hands) and
# flamethrower-tanks.gltf (the tanks worn on the back), each with its .bin, sharing one small
# colour palette (flamethrower-v2.png). With the environment variable PREVIEW set to a path
# prefix, it renders preview pictures (<prefix>-lance-*.png, <prefix>-tanks-*.png).
#
# The lance: a long slender steel tube held with both hands, a pistol grip with its trigger, a
# rear grip with the fuel valve, the igniter at the muzzle, and the hose leaving its underside.
# The tanks: two fuel cylinders and a smaller pressure bottle between them, on a harness frame
# with two shoulder straps, the hose leaving their bottom. Plain military hardware: dark steel,
# field grey, worn army green; no markings.
#
# The lance is built pointing to -Y and turned to +Y at the end (Blender's +Y is glTF's -Z, the
# game's +X, forward); its origin is where the hand holds the pistol grip. The tanks are built
# with the wearer's back towards -Y, Z up, origin between the shoulder blades. In metres.
import math
import os
import sys

import bmesh
import bpy
from mathutils import Matrix, Vector

OUT = sys.argv[sys.argv.index('--') + 1] if '--' in sys.argv else os.path.join(os.path.dirname(os.path.abspath(__file__)), 'flamethrower')
PREVIEW = os.environ.get('PREVIEW')

PALETTE = {
    'steel': (0.20, 0.21, 0.23),       # dark gunmetal
    'steel_light': (0.38, 0.39, 0.40),
    'grey': (0.33, 0.35, 0.33),        # field grey
    'green': (0.25, 0.29, 0.20),       # worn army green
    'green_dark': (0.17, 0.20, 0.14),
    'brass': (0.45, 0.39, 0.25),       # dull brass
    'leather': (0.20, 0.15, 0.10),
    'rubber': (0.06, 0.06, 0.06),
    'soot': (0.03, 0.03, 0.03),
}
NAMES = list(PALETTE)


def cell_uv(name):
    i = NAMES.index(name)
    return ((i % 4) * 0.25 + 0.125, 1.0 - ((i // 4) * 0.25 + 0.125))


bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene


class Builder:
    def __init__(self):
        self.bm = bmesh.new()
        self.uv = self.bm.loops.layers.uv.new('UVMap')

    def paint(self, faces, name, smooth=False):
        uv = cell_uv(name)
        for f in faces:
            f.smooth = smooth
            for loop in f.loops:
                loop[self.uv].uv = uv

    def new_faces(self, before):
        return [f for f in self.bm.faces if f not in before]

    def box(self, lo, hi, colour, bevel=0.0, rot=None, pivot=None):
        before = set(self.bm.faces)
        size = Vector(hi) - Vector(lo)
        centre = (Vector(hi) + Vector(lo)) / 2
        p = Vector(pivot) if pivot else centre
        r = rot if rot is not None else Matrix.Identity(4)
        m = Matrix.Translation(p) @ r @ Matrix.Translation(centre - p) @ Matrix.Diagonal((*size, 1.0))
        ret = bmesh.ops.create_cube(self.bm, size=1.0, matrix=m)
        if bevel > 0.0:
            edges = {e for v in ret['verts'] for e in v.link_edges}
            bmesh.ops.bevel(self.bm, geom=list(edges), offset=bevel, segments=2, affect='EDGES', profile=0.5)
        self.paint(self.new_faces(before), colour)

    def pipe(self, a, b, r0, r1, colour, segments=18, caps=True, cap_colour=None):
        """A cylinder or cone from point a (radius r0) to point b (radius r1)."""
        a, b = Vector(a), Vector(b)
        d = b - a
        before = set(self.bm.faces)
        m = Matrix.Translation((a + b) / 2) @ d.to_track_quat('Z', 'Y').to_matrix().to_4x4()
        bmesh.ops.create_cone(self.bm, cap_ends=caps, cap_tris=False, segments=segments,
                              radius1=r0, radius2=r1, depth=d.length, matrix=m)
        faces = self.new_faces(before)
        self.paint([f for f in faces if len(f.verts) == 4], colour, smooth=True)
        self.paint([f for f in faces if len(f.verts) != 4], cap_colour or colour)

    def ball(self, centre, radius, colour, scale=(1, 1, 1)):
        before = set(self.bm.faces)
        bmesh.ops.create_uvsphere(self.bm, u_segments=16, v_segments=8, radius=radius,
                                  matrix=Matrix.Translation(centre) @ Matrix.Diagonal((*scale, 1.0)))
        self.paint(self.new_faces(before), colour, smooth=True)

    def hose(self, points, radius, colour):
        """A bent hose through the points (short straight pieces)."""
        for p0, p1 in zip(points, points[1:]):
            self.pipe(p0, p1, radius, radius, colour, segments=10, caps=False)
        for p in points[1:-1]:
            self.ball(p, radius, colour)

    def finish(self, name, turn_z=0.0):
        bmesh.ops.remove_doubles(self.bm, verts=self.bm.verts, dist=1e-6)
        if turn_z:
            bmesh.ops.rotate(self.bm, verts=self.bm.verts, cent=(0, 0, 0), matrix=Matrix.Rotation(turn_z, 3, 'Z'))
        self.bm.normal_update()
        mesh = bpy.data.meshes.new(name)
        self.bm.to_mesh(mesh)
        self.bm.free()
        obj = bpy.data.objects.new(name, mesh)
        scene.collection.objects.link(obj)
        return obj


def bezier(p0, p1, p2, p3, n):
    out = []
    for i in range(n + 1):
        t = i / n
        a = (1 - t) ** 3
        b = 3 * (1 - t) ** 2 * t
        c = 3 * (1 - t) * t ** 2
        d = t ** 3
        out.append(Vector(p0) * a + Vector(p1) * b + Vector(p2) * c + Vector(p3) * d)
    return out


# ================================ The lance ================================================
L = Builder()
Z = 0.030                                                     # height of the lance's axis
L.pipe((0, 0.130, Z), (0, -0.600, Z), 0.0135, 0.0135, 'steel')          # the long tube
L.pipe((0, 0.150, Z), (0, 0.130, Z), 0.020, 0.020, 'steel_light')       # rear cap
L.pipe((0, -0.600, Z), (0, -0.640, Z), 0.019, 0.023, 'steel', caps=False)   # muzzle
L.pipe((0, -0.640, Z), (0, -0.610, Z), 0.017, 0.010, 'soot', caps=False)    # its sooty inside
# Igniter at the muzzle: a squat cylinder under the tube with its little guard.
L.pipe((0, -0.560, Z - 0.032), (0, -0.625, Z - 0.032), 0.016, 0.016, 'grey', cap_colour='soot')
L.box((-0.006, -0.600, Z - 0.022), (0.006, -0.585, Z - 0.008), 'steel')
L.box((-0.020, -0.632, Z - 0.054), (0.020, -0.628, Z + 0.010), 'steel')    # guard plate
# Heat guard: a perforated-looking sleeve near the front (rings).
L.pipe((0, -0.300, Z), (0, -0.520, Z), 0.0185, 0.0185, 'grey')
for k in range(6):
    y = -0.315 - k * 0.036
    L.pipe((0, y, Z), (0, y - 0.010, Z), 0.0200, 0.0200, 'steel')
# Front grip: a wooden-looking rubbered handle under the tube, held by the left hand.
L.box((-0.016, -0.265, Z - 0.120), (0.016, -0.225, Z - 0.010), 'rubber', bevel=0.006)
L.box((-0.020, -0.270, Z - 0.022), (0.020, -0.220, Z - 0.006), 'steel', bevel=0.003)   # clamp
# Body at the back: the fuel valve block, the pistol grip and its trigger.
L.box((-0.024, -0.040, Z - 0.026), (0.024, 0.110, Z + 0.024), 'green', bevel=0.008)
L.box((-0.025, 0.020, Z - 0.027), (0.025, 0.060, Z + 0.025), 'green_dark', bevel=0.004)
L.pipe((0.026, 0.060, Z), (0.050, 0.060, Z), 0.011, 0.011, 'brass')        # valve stem
L.pipe((0.050, 0.060, Z), (0.056, 0.060, Z), 0.024, 0.024, 'steel_light')  # valve wheel
LEAN = Matrix.Rotation(math.radians(-16), 4, 'X')
L.box((-0.017, 0.040, -0.120), (0.017, 0.092, Z - 0.020), 'rubber', bevel=0.008, rot=LEAN, pivot=(0, 0.066, Z - 0.020))
L.box((-0.004, 0.000, Z - 0.068), (0.004, 0.010, Z - 0.024), 'steel', rot=Matrix.Rotation(math.radians(12), 4, 'X'))  # trigger
L.box((-0.005, -0.012, Z - 0.076), (0.005, 0.040, Z - 0.068), 'steel')      # guard, bottom
L.box((-0.005, -0.012, Z - 0.076), (0.005, -0.004, Z - 0.026), 'steel')     # guard, front
# Rear grip: a tube along the back with a leather wrap.
L.pipe((0, 0.110, Z - 0.015), (0, 0.170, Z - 0.060), 0.013, 0.013, 'leather')
# The hose: from the body's underside, down and back towards the bottom of the screen.
pts = bezier((0, 0.090, Z - 0.030), (0, 0.120, Z - 0.140), (0.030, 0.200, Z - 0.240), (0.060, 0.320, Z - 0.380), 9)
L.hose(pts, 0.011, 'rubber')
lance = L.finish('FlamethrowerLance', turn_z=math.pi)

# ================================ The tanks ================================================
T = Builder()
# The wearer's back is towards -Y (the tanks hang behind it, at positive Y).
for x in (-0.115, 0.115):
    T.pipe((x, 0.120, -0.280), (x, 0.120, 0.200), 0.085, 0.085, 'green')          # fuel tank
    T.ball((x, 0.120, 0.200), 0.085, 'green', scale=(1, 1, 0.55))                 # domed top
    T.ball((x, 0.120, -0.280), 0.085, 'green_dark', scale=(1, 1, 0.45))           # domed bottom
    T.pipe((x, 0.120, 0.235), (x, 0.120, 0.262), 0.020, 0.020, 'brass')           # filler cap
    T.pipe((x, 0.120, 0.262), (x, 0.120, 0.272), 0.028, 0.028, 'steel')
    for z in (-0.180, 0.110):                                                     # tank bands
        T.pipe((x, 0.120, z), (x, 0.120, z + 0.022), 0.088, 0.088, 'steel')
# Pressure bottle between them, set back.
T.pipe((0, 0.195, -0.220), (0, 0.195, 0.080), 0.052, 0.052, 'grey')
T.ball((0, 0.195, 0.080), 0.052, 'grey', scale=(1, 1, 0.6))
T.ball((0, 0.195, -0.220), 0.052, 'grey', scale=(1, 1, 0.5))
T.pipe((0, 0.195, 0.110), (0, 0.195, 0.135), 0.014, 0.014, 'brass')
T.pipe((0, 0.195, 0.135), (0, 0.195, 0.142), 0.022, 0.022, 'steel_light')        # its valve wheel
# Harness frame: a back plate against the wearer, crossbars, shoulder straps.
T.box((-0.200, 0.020, -0.300), (0.200, 0.035, 0.180), 'steel', bevel=0.006)
for z in (-0.230, 0.050):
    T.box((-0.215, 0.030, z), (0.215, 0.140, z + 0.030), 'steel', bevel=0.005)
for x in (-0.095, 0.095):                                                         # shoulder straps
    strap = bezier((x, 0.020, 0.170), (x, -0.020, 0.300), (x, -0.160, 0.290), (x, -0.200, 0.120), 8)
    for p0, p1 in zip(strap, strap[1:]):
        d = p1 - p0
        m = Matrix.Translation((p0 + p1) / 2) @ d.to_track_quat('Z', 'X').to_matrix().to_4x4()
        before = set(T.bm.faces)
        bmesh.ops.create_cube(T.bm, size=1.0, matrix=m @ Matrix.Diagonal((0.050, 0.008, d.length * 1.08, 1.0)))
        T.paint(T.new_faces(before), 'leather')
    T.box((x - 0.030, -0.215, -0.120), (x + 0.030, -0.200, 0.130), 'leather')     # down the chest
# Pipes joining the tanks at the bottom, and the hose leaving to the right hip.
T.pipe((-0.115, 0.120, -0.330), (0.115, 0.120, -0.330), 0.016, 0.016, 'steel')
T.pipe((0, 0.120, -0.330), (0, 0.120, -0.370), 0.020, 0.020, 'brass')
hose = bezier((0, 0.120, -0.370), (0.050, 0.100, -0.470), (0.180, 0.000, -0.430), (0.230, -0.120, -0.300), 10)
T.hose(hose, 0.016, 'rubber')
tanks = T.finish('FlamethrowerTanks')

# ============================ Palette, material, export ===================================
models = os.path.join(OUT, 'models')
os.makedirs(models, exist_ok=True)
img = bpy.data.images.new('flamethrower-v2', 64, 64, alpha=False)
pixels = [0.0] * (64 * 64 * 4)
for py in range(64):
    for px in range(64):
        i = (3 - py // 16) * 4 + px // 16
        r, g, b = PALETTE[NAMES[i]] if i < len(NAMES) else (1.0, 0.0, 1.0)
        pixels[(py * 64 + px) * 4:(py * 64 + px) * 4 + 4] = (r, g, b, 1.0)
img.pixels.foreach_set(pixels)
img.update()
img.filepath_raw = os.path.join(models, 'flamethrower-v2.png')
img.file_format = 'PNG'
img.save()
mat = bpy.data.materials.new('FlamethrowerV2')
mat.use_nodes = True
tex = mat.node_tree.nodes.new('ShaderNodeTexImage')
tex.image = img
tex.interpolation = 'Closest'
bsdf = next(n for n in mat.node_tree.nodes if n.type == 'BSDF_PRINCIPLED')
mat.node_tree.links.new(tex.outputs['Color'], bsdf.inputs['Base Color'])
for o in (lance, tanks):
    o.data.materials.append(mat)


def export(obj, name):
    for o in scene.objects:
        o.select_set(o == obj)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.export_scene.gltf(filepath=os.path.join(models, name + '.gltf'), export_format='GLTF_SEPARATE',
                              use_selection=True, export_yup=True, export_apply=True,
                              export_texcoords=True, export_normals=True, export_materials='EXPORT',
                              export_image_format='AUTO', export_keep_originals=True,
                              export_animations=False, export_cameras=False, export_lights=False)


export(lance, 'flamethrower-lance')
export(tanks, 'flamethrower-tanks')


def render(path, objs, location, target, ortho_scale, width=1280, height=720):
    for o in scene.objects:
        if o.type == 'MESH':
            o.hide_render = o not in objs
    cam_data = bpy.data.cameras.new('cam')
    cam_data.type = 'ORTHO'
    cam_data.ortho_scale = ortho_scale
    cam = bpy.data.objects.new('cam', cam_data)
    scene.collection.objects.link(cam)
    cam.location = location
    cam.rotation_euler = (Vector(target) - Vector(location)).to_track_quat('-Z', 'Y').to_euler()
    scene.camera = cam
    scene.render.engine = 'BLENDER_WORKBENCH'
    scene.display.shading.light = 'STUDIO'
    scene.display.shading.color_type = 'TEXTURE'
    scene.display.shading.show_object_outline = True
    scene.display.render_aa = '16'
    scene.render.film_transparent = False
    scene.world = scene.world or bpy.data.worlds.new('w')
    scene.render.resolution_x = width
    scene.render.resolution_y = height
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = 'PNG'
    scene.view_settings.view_transform = 'Standard'
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)
    bpy.data.objects.remove(cam)


if PREVIEW:
    render(PREVIEW + '-lance-side.png', [lance], (1.2, 0.20, -0.06), (0.0, 0.20, -0.06), 0.95)
    render(PREVIEW + '-lance-fps.png', [lance], (0.16, -0.40, 0.16), (0.0, 0.35, -0.02), 0.75)
    render(PREVIEW + '-tanks-back.png', [tanks], (0.0, 1.5, -0.08), (0.0, 0.0, -0.08), 1.25)
    render(PREVIEW + '-tanks-side.png', [tanks], (1.5, 0.05, -0.08), (0.0, 0.05, -0.08), 1.25)
    render(PREVIEW + '-tanks-3q.png', [tanks], (1.0, 1.1, 0.45), (0.0, 0.05, -0.10), 1.35)
print('lance %d faces, tanks %d faces' % (len(lance.data.polygons), len(tanks.data.polygons)))

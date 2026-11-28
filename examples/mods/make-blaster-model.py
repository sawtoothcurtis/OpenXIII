# Builds the Blaster example's 3D model with Blender, without opening a window:
#
#   blender -b --python make-blaster-model.py [-- <mod folder>]
#
# It writes blaster.gltf, blaster.bin and blaster.png (a small colour palette) into
# <mod folder>/models and the HUD icon into <mod folder>/textures/blaster-icon.png (the mod
# folder is "blaster-gun", next to this script, by default). With the environment variable PREVIEW
# set to a path prefix, it also renders three preview pictures (<prefix>-side.png…).
#
# The gun is built pointing to -Y and turned to +Y at the end: Blender's +Y is glTF's -Z, the
# game's +X (forward). Z up, in metres; its origin is where the hand holds it (top of the grip).
import math
import os
import sys

import bmesh
import bpy
from mathutils import Matrix, Vector

OUT = sys.argv[sys.argv.index('--') + 1] if '--' in sys.argv else os.path.join(os.path.dirname(os.path.abspath(__file__)), 'blaster-gun')
PREVIEW = os.environ.get('PREVIEW')

# The palette: one flat colour per 16 x 16 cell of a 64 x 64 image. Every face of the model is
# mapped to the middle of one cell, so the model needs no unwrapping.
PALETTE = {
    'dark': (0.17, 0.19, 0.24),
    'steel': (0.47, 0.50, 0.57),
    'light': (0.80, 0.82, 0.86),
    'red': (0.84, 0.22, 0.10),
    'glow': (0.30, 0.88, 1.00),
    'grip': (0.07, 0.07, 0.08),
    'yellow': (0.96, 0.76, 0.12),
    'core': (0.86, 1.00, 1.00),
}
NAMES = list(PALETTE)


def cell_uv(name):
    i = NAMES.index(name)
    return ((i % 4) * 0.25 + 0.125, 1.0 - ((i // 4) * 0.25 + 0.125))


bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene

bm = bmesh.new()
uv_layer = bm.loops.layers.uv.new('UVMap')


def paint(geom, name, smooth=False):
    uv = cell_uv(name)
    for f in geom:
        if isinstance(f, bmesh.types.BMFace):
            f.smooth = smooth
            for loop in f.loops:
                loop[uv_layer].uv = uv


def new_faces(before):
    return [f for f in bm.faces if f not in before]


def box(lo, hi, colour, bevel=0.0, rot_x=0.0, pivot=None):
    """A box from corner lo to corner hi, then turned around the X axis through pivot (its own
    centre by default)."""
    before = set(bm.faces)
    size = Vector(hi) - Vector(lo)
    centre = (Vector(hi) + Vector(lo)) / 2
    p = Vector(pivot) if pivot else centre
    m = (Matrix.Translation(p) @ Matrix.Rotation(rot_x, 4, 'X') @ Matrix.Translation(centre - p)
         @ Matrix.Diagonal((*size, 1.0)))
    ret = bmesh.ops.create_cube(bm, size=1.0, matrix=m)
    if bevel > 0.0:
        edges = {e for v in ret['verts'] for e in v.link_edges}
        bmesh.ops.bevel(bm, geom=list(edges), offset=bevel, segments=2, affect='EDGES', profile=0.5)
    paint(new_faces(before), colour)


def tube(y0, r0, y1, r1, colour, z=0.0, x=0.0, segments=20, caps=True, cap_colour=None):
    """A cylinder or cone along Y, from (y0, radius r0) to (y1, radius r1)."""
    before = set(bm.faces)
    # create_cone builds along Z: turn Z onto -Y so that "y0" is its base.
    length = abs(y1 - y0)
    m = Matrix.Translation((x, (y0 + y1) / 2, z)) @ Matrix.Rotation(math.radians(90 if y1 < y0 else -90), 4, 'X')
    bmesh.ops.create_cone(bm, cap_ends=caps, cap_tris=False, segments=segments,
                          radius1=r0, radius2=r1, depth=length, matrix=m)
    faces = new_faces(before)
    side = [f for f in faces if len(f.verts) == 4]
    ends = [f for f in faces if len(f.verts) != 4]
    paint(side, colour, smooth=True)
    paint(ends, cap_colour or colour)


def ball(centre, radius, colour):
    before = set(bm.faces)
    bmesh.ops.create_uvsphere(bm, u_segments=14, v_segments=8, radius=radius,
                              matrix=Matrix.Translation(centre))
    paint(new_faces(before), colour, smooth=True)


# --- Body ---------------------------------------------------------------------------------
box((-0.028, -0.085, 0.000), (0.028, 0.130, 0.076), 'dark', bevel=0.012)
# Red side plates and the glowing power lines running along them.
for side in (-1, 1):
    box((side * 0.026, -0.060, 0.016), (side * 0.0315, 0.100, 0.060), 'red', bevel=0.0018)
    box((side * 0.030, -0.045, 0.032), (side * 0.0340, 0.085, 0.044), 'glow')
    for k in range(3):                     # three cooling slots behind the power line
        box((side * 0.030, 0.060 + k * 0.012, 0.049), (side * 0.0335, 0.066 + k * 0.012, 0.056), 'dark')
# Top: a light rail carrying the energy cell (a glowing tube between two collars).
box((-0.015, -0.070, 0.074), (0.015, 0.112, 0.088), 'light', bevel=0.004)
tube(0.085, 0.0125, -0.040, 0.0125, 'glow', z=0.100, cap_colour='core')
tube(0.095, 0.016, 0.080, 0.016, 'steel', z=0.100)
tube(-0.035, 0.016, -0.050, 0.016, 'steel', z=0.100)
tube(0.015, 0.0145, 0.005, 0.0145, 'steel', z=0.100)
# Rear sight fin and the round gauge on the back, facing the player.
box((-0.004, 0.100, 0.086), (0.004, 0.128, 0.112), 'red', bevel=0.0015)
tube(0.128, 0.022, 0.138, 0.019, 'steel', z=0.040)
tube(0.137, 0.014, 0.142, 0.012, 'glow', z=0.040, cap_colour='core')

# --- Barrel -------------------------------------------------------------------------------
tube(-0.080, 0.031, -0.108, 0.020, 'steel', z=0.040)               # shoulder
tube(-0.105, 0.016, -0.250, 0.016, 'light', z=0.040)               # barrel
for k in range(4):                                                  # cooling rings
    y = -0.128 - k * 0.026
    tube(y, 0.027, y - 0.009, 0.027, 'red', z=0.040)
tube(-0.245, 0.018, -0.288, 0.034, 'dark', z=0.040, caps=False)     # muzzle flare (open)
tube(-0.288, 0.034, -0.252, 0.012, 'steel', z=0.040, caps=False)    # its inside
ball((0.0, -0.262, 0.040), 0.014, 'core')                           # the emitter in the muzzle
tube(-0.284, 0.0345, -0.292, 0.0345, 'yellow', z=0.040, caps=False)  # muzzle lip
# Under-barrel stabiliser with a yellow tip.
box((-0.011, -0.205, -0.004), (0.011, -0.080, 0.016), 'dark', bevel=0.004)
box((-0.0115, -0.215, -0.002), (0.0115, -0.203, 0.014), 'yellow', bevel=0.002)

# --- Grip, trigger ------------------------------------------------------------------------
LEAN = math.radians(-14)
TOP = (0.0, 0.090, 0.0)                                             # the grip leans back around this point
box((-0.020, 0.060, -0.112), (0.020, 0.120, 0.010), 'grip', bevel=0.008, rot_x=LEAN, pivot=TOP)
box((-0.022, 0.057, -0.124), (0.022, 0.123, -0.110), 'red', bevel=0.003, rot_x=LEAN, pivot=TOP)
for k in range(4):                                                  # ridges on the back of the grip
    z = -0.030 - k * 0.018
    box((-0.021, 0.110, z - 0.005), (0.021, 0.123, z + 0.005), 'steel', bevel=0.002, rot_x=LEAN, pivot=TOP)
box((-0.005, -0.020, -0.046), (0.005, -0.010, 0.004), 'steel')      # guard, front
box((-0.005, -0.020, -0.052), (0.005, 0.066, -0.044), 'steel')      # guard, bottom
box((-0.004, 0.020, -0.032), (0.004, 0.029, 0.004), 'yellow', rot_x=math.radians(12))

bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-6)
bmesh.ops.rotate(bm, verts=bm.verts, cent=(0.0, 0.0, 0.0), matrix=Matrix.Rotation(math.pi, 3, 'Z'))
bm.normal_update()
mesh = bpy.data.meshes.new('Blaster')
bm.to_mesh(mesh)
bm.free()
obj = bpy.data.objects.new('Blaster', mesh)
scene.collection.objects.link(obj)

# --- Palette texture and material ---------------------------------------------------------
models = os.path.join(OUT, 'models')
os.makedirs(models, exist_ok=True)
img = bpy.data.images.new('blaster', 64, 64, alpha=False)
pixels = [0.0] * (64 * 64 * 4)
for py in range(64):
    for px in range(64):
        i = (3 - py // 16) * 4 + px // 16      # image rows go bottom to top
        r, g, b = PALETTE[NAMES[i]] if i < len(NAMES) else (1.0, 0.0, 1.0)
        pixels[(py * 64 + px) * 4:(py * 64 + px) * 4 + 4] = (r, g, b, 1.0)
img.pixels.foreach_set(pixels)
img.update()
img.filepath_raw = os.path.join(models, 'blaster.png')
img.file_format = 'PNG'
img.save()

mat = bpy.data.materials.new('Blaster')
mat.use_nodes = True
nodes = mat.node_tree.nodes
tex = nodes.new('ShaderNodeTexImage')
tex.image = img
tex.interpolation = 'Closest'
bsdf = next(n for n in nodes if n.type == 'BSDF_PRINCIPLED')
mat.node_tree.links.new(tex.outputs['Color'], bsdf.inputs['Base Color'])
mesh.materials.append(mat)

bpy.context.view_layer.objects.active = obj
obj.select_set(True)
bpy.ops.export_scene.gltf(filepath=os.path.join(models, 'blaster.gltf'), export_format='GLTF_SEPARATE',
                          use_selection=True, export_yup=True, export_apply=True,
                          export_texcoords=True, export_normals=True, export_materials='EXPORT',
                          export_image_format='AUTO', export_keep_originals=True,
                          export_animations=False, export_cameras=False, export_lights=False)


# --- Pictures: the HUD icon (side view) and an optional preview ----------------------------
def render(path, location, target, ortho_scale, width, height, flat, samples=16):
    cam_data = bpy.data.cameras.new('cam')
    cam_data.type = 'ORTHO'
    cam_data.ortho_scale = ortho_scale
    cam = bpy.data.objects.new('cam', cam_data)
    scene.collection.objects.link(cam)
    cam.location = location
    direction = Vector(target) - Vector(location)
    cam.rotation_euler = direction.to_track_quat('-Z', 'Y').to_euler()
    scene.camera = cam
    scene.render.engine = 'BLENDER_WORKBENCH'
    shading = scene.display.shading
    shading.light = 'FLAT' if flat else 'STUDIO'
    shading.color_type = 'TEXTURE'
    shading.show_object_outline = not flat
    scene.display.render_aa = str(samples)
    scene.render.film_transparent = True
    scene.render.resolution_x = width
    scene.render.resolution_y = height
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = 'PNG'
    scene.render.image_settings.color_mode = 'RGBA'
    scene.view_settings.view_transform = 'Standard'
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)
    bpy.data.objects.remove(cam)


textures = os.path.join(OUT, 'textures')
os.makedirs(textures, exist_ok=True)
# Side view with the barrel pointing to the right, as on the game's icons.
render(os.path.join(textures, 'blaster-icon.png'), (1.0, 0.075, -0.005), (0.0, 0.075, -0.005),
       0.47, 64, 32, flat=True)
if PREVIEW:
    render(PREVIEW + '-side.png', (1.0, 0.075, -0.005), (0.0, 0.075, -0.005), 0.5, 1280, 720, flat=False)
    render(PREVIEW + '-fps.png', (-0.25, -0.55, 0.30), (0.0, 0.08, 0.03), 0.5, 1280, 720, flat=False)
    render(PREVIEW + '-front.png', (0.45, 0.75, 0.25), (0.0, 0.08, 0.02), 0.5, 1280, 720, flat=False)
print('blaster: %d vertices, %d faces' % (len(mesh.vertices), len(mesh.polygons)))

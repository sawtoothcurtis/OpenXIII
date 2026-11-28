package org.openxiii;

import android.content.Context;
import android.content.SharedPreferences;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.List;

/**
 * The on-screen controls: what they are, where they sit and how big they are.
 * Positions are fractions of the overlay size so a layout survives rotation and
 * other screens. Saved as JSON in SharedPreferences.
 */
public final class TouchLayout {
    public static final int MAX_CONTROLS = 64;
    public static final float MIN_SCALE = 0.5f;
    public static final float MAX_SCALE = 2.0f;
    public static final float MIN_OPACITY = 0.2f;
    public static final float MAX_OPACITY = 1.0f;
    public static final float MIN_LOOK_SPEED = 0.25f;
    public static final float MAX_LOOK_SPEED = 3.0f;

    private static final String PREFS = "openxiii_touch";
    private static final String KEY_LAYOUT = "layout";
    private static final int VERSION = 1;

    /** What a control sends to the game. */
    public enum Type { STICK, KEY, MOUSE, WHEEL }

    /**
     * Every kind of control. Keys follow DefUser.ini (Space Jump, C Duck,
     * R Reload, E Grab, Q QuickHeal, F/G inventory, Escape menu, LShift walk).
     * The look stick is hidden by default: sliding a finger on the screen looks
     * around (it can be added back in the Edit panel; a saved layout keeps it).
     */
    public enum Kind {
        //          label      type        code  size(dp) x      y      shown
        MOVE_STICK("Move",     Type.STICK, 0,    140, 0.15f, 0.70f, true),
        LOOK_STICK("Look",     Type.STICK, 1,    140, 0.60f, 0.70f, false),
        FIRE(      "Fire",     Type.MOUSE, 0,    88,  0.90f, 0.58f, true),
        ALT_FIRE(  "Alt fire", Type.MOUSE, 1,    64,  0.77f, 0.62f, true),
        MIDDLE(    "Middle",   Type.MOUSE, 2,    56,  0.66f, 0.40f, false),
        JUMP(      "Jump",     Type.KEY,   0x20, 64,  0.93f, 0.87f, true),
        CROUCH(    "Crouch",   Type.KEY,   0x43, 64,  0.82f, 0.87f, true),
        RELOAD(    "Reload",   Type.KEY,   0x52, 60,  0.71f, 0.87f, true),
        GRAB(      "Action",   Type.KEY,   0x45, 60,  0.90f, 0.33f, true),
        HEAL(      "Heal",     Type.KEY,   0x51, 52,  0.21f, 0.12f, true),
        INV_PREV(  "Item -",   Type.KEY,   0x46, 52,  0.31f, 0.12f, true),
        INV_NEXT(  "Item +",   Type.KEY,   0x47, 52,  0.39f, 0.12f, true),
        WEAPON_PREV("Weapon -", Type.WHEEL, -1,  52,  0.70f, 0.12f, true),
        WEAPON_NEXT("Weapon +", Type.WHEEL, 1,   52,  0.79f, 0.12f, true),
        MENU(      "Menu",     Type.KEY,   0x1B, 48,  0.06f, 0.11f, true),
        ENTER(     "Enter",    Type.KEY,   0x0D, 52,  0.94f, 0.12f, false),
        WALK(      "Walk",     Type.KEY,   0xA0, 52,  0.06f, 0.45f, false);

        public final String label;
        public final Type type;
        public final int code;
        public final float sizeDp;
        final float defX, defY;
        final boolean defShown;

        Kind(String label, Type type, int code, float sizeDp, float defX, float defY, boolean defShown) {
            this.label = label;
            this.type = type;
            this.code = code;
            this.sizeDp = sizeDp;
            this.defX = defX;
            this.defY = defY;
            this.defShown = defShown;
        }

        public boolean isStick() {
            return type == Type.STICK;
        }
    }

    /** One control on screen: centre as fractions of the overlay, size factor. */
    public static final class Control {
        public final Kind kind;
        public float x, y;
        public float scale;

        public Control(Kind kind, float x, float y, float scale) {
            this.kind = kind;
            this.x = x;
            this.y = y;
            this.scale = scale;
        }

        /** Diameter in pixels. */
        public float diameter(float density) {
            return kind.sizeDp * scale * density;
        }
    }

    public final List<Control> controls = new ArrayList<>();
    /** Opacity of every control and of the Edit button; the minimum (MIN_OPACITY) by default. */
    public float opacity = MIN_OPACITY;
    /** Camera speed of a finger sliding on the screen (1 = default). */
    public float lookSpeed = 1f;
    /** Invincible player (the game's God cheat), for testing levels. */
    public boolean godMode = false;
    /** Move speed multiplier (1, 2 or 3) and infinite ammo, for testing levels. */
    public int moveSpeed = 1;
    public boolean infiniteAmmo = false;
    /** The move stick appears where the thumb lands in the lower left. */
    public boolean floatingStick = true;
    /** A finger that slides from a held button also aims (sent to touch()). */
    public boolean buttonsAim = true;

    public static TouchLayout defaults() {
        TouchLayout l = new TouchLayout();
        for (Kind k : Kind.values()) {
            if (k.defShown) l.controls.add(new Control(k, k.defX, k.defY, 1f));
        }
        return l;
    }

    public Control find(Kind kind) {
        for (Control c : controls) if (c.kind == kind) return c;
        return null;
    }

    /** Adds a control of this kind, or returns null (sticks are single, 64 at most). */
    public Control add(Kind kind) {
        if (controls.size() >= MAX_CONTROLS) return null;
        if (kind.isStick() && find(kind) != null) return null;
        Control c = find(kind) == null
                ? new Control(kind, kind.defX, kind.defY, 1f)
                : new Control(kind, 0.5f, 0.3f, 1f);
        controls.add(c);
        return c;
    }

    public static float clamp(float v, float lo, float hi) {
        return v < lo ? lo : (v > hi ? hi : v);
    }

    // --- Saving --------------------------------------------------------------

    public String toJson() {
        try {
            JSONObject o = new JSONObject();
            o.put("version", VERSION);
            o.put("opacity", (double) opacity);
            o.put("lookSpeed", (double) lookSpeed);
            o.put("godMode", godMode);
            o.put("moveSpeed", moveSpeed);
            o.put("infiniteAmmo", infiniteAmmo);
            o.put("floatingStick", floatingStick);
            o.put("buttonsAim", buttonsAim);
            JSONArray a = new JSONArray();
            for (Control c : controls) {
                JSONObject j = new JSONObject();
                j.put("kind", c.kind.name());
                j.put("x", (double) c.x);
                j.put("y", (double) c.y);
                j.put("scale", (double) c.scale);
                a.put(j);
            }
            o.put("controls", a);
            return o.toString();
        } catch (JSONException e) {
            throw new IllegalStateException(e);
        }
    }

    /** Reads a layout; unknown controls are skipped, broken files give null. */
    public static TouchLayout fromJson(String s) {
        try {
            JSONObject o = new JSONObject(s);
            if (o.optInt("version", 0) != VERSION) return null;
            TouchLayout l = new TouchLayout();
            l.opacity = clamp((float) o.optDouble("opacity", l.opacity), MIN_OPACITY, MAX_OPACITY);
            l.lookSpeed = clamp((float) o.optDouble("lookSpeed", l.lookSpeed), MIN_LOOK_SPEED, MAX_LOOK_SPEED);
            l.godMode = o.optBoolean("godMode", l.godMode);
            l.moveSpeed = Math.max(1, Math.min(3, o.optInt("moveSpeed", l.moveSpeed)));
            l.infiniteAmmo = o.optBoolean("infiniteAmmo", l.infiniteAmmo);
            l.floatingStick = o.optBoolean("floatingStick", l.floatingStick);
            l.buttonsAim = o.optBoolean("buttonsAim", l.buttonsAim);
            JSONArray a = o.getJSONArray("controls");
            for (int i = 0; i < a.length() && l.controls.size() < MAX_CONTROLS; i++) {
                JSONObject j = a.getJSONObject(i);
                Kind kind;
                try {
                    kind = Kind.valueOf(j.getString("kind"));
                } catch (IllegalArgumentException e) {
                    continue;
                }
                if (kind.isStick() && l.find(kind) != null) continue;
                l.controls.add(new Control(kind,
                        clamp((float) j.getDouble("x"), 0f, 1f),
                        clamp((float) j.getDouble("y"), 0f, 1f),
                        clamp((float) j.getDouble("scale"), MIN_SCALE, MAX_SCALE)));
            }
            return l;
        } catch (JSONException e) {
            return null;
        }
    }

    public static TouchLayout load(Context ctx) {
        String s = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY_LAYOUT, null);
        if (s != null) {
            TouchLayout l = fromJson(s);
            if (l != null) return l;
        }
        return defaults();
    }

    public void save(Context ctx) {
        SharedPreferences.Editor e = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit();
        e.putString(KEY_LAYOUT, toJson());
        e.apply();
    }
}

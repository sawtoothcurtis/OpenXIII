package org.openxiii;

/**
 * Touch control input to the game (crates/openxiii-viewer/src/touch_controls.rs).
 * Callable from the UI thread: the game queues the calls and applies them on
 * the next frame.
 */
public final class NativeInput {
    private NativeInput() {}

    /** Windows key (VK code: 0x57 W, 0x20 Space, 0x1B Escape, 0x0D Enter…). */
    public static native void key(int vk, boolean down);

    /** Character typed on the phone's keyboard in a menu text field (the PC's WM_CHAR). */
    public static native void character(int ch);

    /**
     * Menu text field that has the focus: 0 none, otherwise (field + 1) * 2, + 1 if it is masked
     * (password).
     */
    public static native int textField();

    /** Text of the menu text field that has the focus ("" without one). */
    public static native String textFieldText();

    /** Mouse button: 0 left (fire), 1 right, 2 middle. */
    public static native void mouseButton(int button, boolean down);

    /** Wheel: +1 next weapon, -1 previous. */
    public static native void wheel(int steps);

    /** Stick 0 move, 1 look; x to the right, y down, from -1 to 1. */
    public static native void stick(int which, float x, float y);

    /** Finger outside the controls, in screen pixels: action 0 down, 1 move, 2 up. */
    public static native void touch(int action, int id, float x, float y);

    /**
     * Startup: -2 while the game loads, 0 to 100 while it makes the menus' pictures (first launch
     * of a new version), -1 once the game shows its own image.
     */
    public static native int loading();

    /** True when the game captures the mouse (playing); false in the menus and while loading. */
    public static native boolean captured();

    /**
     * Where the game image is shown on the screen (screen pixels), to scale touch() to the game
     * image: it can be narrower than the screen, between black bars.
     */
    public static native void viewRect(int left, int top, int width, int height);

    /** The phone's screen in landscape (pixels): the game's resolutions are made from it. */
    public static native void screenSize(int width, int height);

    /**
     * Size the game image is to take (width << 32 | height), set by the game's Video menu or its
     * ini at startup; 0 when unchanged since the last call.
     */
    public static native long wantedSurface();

    /** Camera speed of a finger sliding on the screen (1 = normal). */
    public static native void lookSpeed(float speed);

    /** Console versions' aim assist (iAutoAimMode): on or off. */
    public static native void setAutoAim(boolean on);

    /**
     * Gamepad in slot (0…3): connected or not, sticks (−1…1, Y down), triggers
     * (0…1) and buttons (bits: A, B, X, Y, LB, RB, Back, Start, left stick, right stick, then
     * the d-pad up, down, left, right).
     */
    public static native void pad(int slot, boolean connected, float lx, float ly, float rx, float ry,
            float lt, float rt, int buttons);

    /** Rumble of the gamepad in slot: large motor × 256 + small motor (0…255). */
    public static native int vibration(int slot);

    /** The first player has a gamepad (in split screen, not only the other players). */
    public static native boolean firstPlayerGamepad();

    /** Player's move speed: 1, 2 or 3 times normal (for testing levels). */
    public static native void setMoveSpeed(float mult);

    /** Infinite ammo (magazine and reserve always full). */
    public static native void setInfiniteAmmo(boolean on);

    /** Player invincibility (the game's God cheat), for testing levels. */
    public static native void setGodMode(boolean on);

    /** "cheats" build: true if the Edit panel shows the cheats (otherwise the normal build). */
    public static native boolean cheats();

    /** Player's aim assist: 0 off, 1 on, 2 no game loaded. */
    public static native int autoAim();
}

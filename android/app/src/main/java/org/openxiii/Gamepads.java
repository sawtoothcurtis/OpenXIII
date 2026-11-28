package org.openxiii;

import android.content.Context;
import android.hardware.input.InputManager;
import android.os.Build;
import android.os.CombinedVibration;
import android.os.Handler;
import android.os.Looper;
import android.os.VibrationEffect;
import android.os.Vibrator;
import android.os.VibratorManager;
import android.util.Log;
import android.util.SparseIntArray;
import android.view.InputDevice;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.widget.Toast;

/**
 * Gamepads (Bluetooth, USB…) to the game (crates/openxiii-viewer/src/pad_android.rs): up to
 * 4 slots, given to a gamepad when it is connected (at start or while playing, InputManager)
 * or when it first sends an event, and freed when it is removed.
 */
final class Gamepads implements InputManager.InputDeviceListener {
    // Buttons, in DirectInput order for an XInput gamepad (pad_android.rs), then the d-pad.
    private static final int A = 1, B = 1 << 1, X = 1 << 2, Y = 1 << 3, LB = 1 << 4, RB = 1 << 5,
            BACK = 1 << 6, START = 1 << 7, THUMB_L = 1 << 8, THUMB_R = 1 << 9,
            UP = 1 << 10, DOWN = 1 << 11, LEFT = 1 << 12, RIGHT = 1 << 13;

    private static final class Pad {
        int deviceId;
        /** Device that rumbles: the gamepad itself, or another device of the same physical
         *  gamepad (same descriptor, or same vendor and product) that has the motors. */
        int vibratorId;
        float lx, ly, rx, ry, lt, rt;
        int keys, hat;
    }

    private final Pad[] slots = new Pad[4];
    /** deviceId → place. */
    private final SparseIntArray slotOf = new SparseIntArray();
    private final InputManager input;
    private final Context context;
    /** Rumble of each slot (large motor × 256 + small), read from the game every 30 ms. */
    private final int[] vibration = new int[4];
    /** Rumble refusal already written to the log, per slot. */
    private final boolean[] rumbleRefused = new boolean[4];
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final Runnable rumbleTick = new Runnable() {
        @Override
        public void run() {
            for (int i = 0; i < slots.length; i++) rumble(i);
            announce();
            handler.postDelayed(this, 30);
        }
    };

    /** Told on the UI thread when the first player gets a gamepad or loses it (in split screen,
     *  the other players' gamepads leave the touch controls to the first player). */
    interface Listener {
        void onGamepadsChanged(boolean any);
    }

    private Listener listener;
    /** Last state given to the listener. */
    private boolean announced;

    /** At least one gamepad holds a slot. */
    boolean anyConnected() {
        for (Pad p : slots) if (p != null) return true;
        return false;
    }

    /** A gamepad drives the first player: any gamepad, except in split screen where the game
     *  says whether one of them is the first player's (pad_android.rs). */
    private boolean firstPlayerHasGamepad() {
        if (!anyConnected()) return false;
        try {
            return NativeInput.firstPlayerGamepad();
        } catch (UnsatisfiedLinkError e) {
            return true;
        }
    }

    /** Sets the listener and tells it the current state at once (gamepad already connected). */
    void setListener(Listener l) {
        listener = l;
        announced = firstPlayerHasGamepad();
        if (l != null) l.onGamepadsChanged(announced);
    }

    private void announce() {
        boolean any = firstPlayerHasGamepad();
        if (any == announced || listener == null) return;
        announced = any;
        listener.onGamepadsChanged(any);
    }

    private void slotsChanged() {
        handler.post(this::announce);
    }

    Gamepads(Context context) {
        this.context = context;
        input = (InputManager) context.getSystemService(Context.INPUT_SERVICE);
        input.registerInputDeviceListener(this, new Handler(Looper.getMainLooper()));
        for (int id : InputDevice.getDeviceIds()) onInputDeviceAdded(id);
        handler.postDelayed(rumbleTick, 30);
    }

    void close() {
        handler.removeCallbacks(rumbleTick);
        for (int i = 0; i < slots.length; i++) stopRumble(i);
        input.unregisterInputDeviceListener(this);
    }

    /**
     * Rumble of the gamepad in slot i (pad_android.rs, the VM's force_feedback.rs): two motors
     * when the gamepad has two (Android 12 and later), otherwise the stronger of the two. A 60 ms
     * pulse started again every 30 ms while the game asks for a force.
     */
    private void rumble(int i) {
        int v;
        try {
            v = NativeInput.vibration(i);
        } catch (UnsatisfiedLinkError e) {
            return;
        }
        if (slots[i] == null) v = 0;
        if (v == 0) {
            if (vibration[i] != 0) stopRumble(i);
            vibration[i] = 0;
            return;
        }
        vibration[i] = v;
        InputDevice d = InputDevice.getDevice(slots[i].vibratorId);
        if (d == null) d = InputDevice.getDevice(slots[i].deviceId);
        if (d == null) return;
        int large = (v >> 8) & 0xff, small = v & 0xff;
        try {
            if (Build.VERSION.SDK_INT >= 31) {
                VibratorManager vm = d.getVibratorManager();
                int[] ids = vm.getVibratorIds();
                if (ids.length >= 2) {
                    CombinedVibration.ParallelCombination c = CombinedVibration.startParallel();
                    if (large > 0) c.addVibrator(ids[0], VibrationEffect.createOneShot(60, large));
                    if (small > 0) c.addVibrator(ids[1], VibrationEffect.createOneShot(60, small));
                    vm.vibrate(c.combine());
                    return;
                }
                if (ids.length == 1) {
                    // A single motor: the manager's own path (the documented one on Android 12+).
                    vm.vibrate(CombinedVibration.createParallel(VibrationEffect.createOneShot(60, Math.max(large, small))));
                    return;
                }
            }
            Vibrator vib = d.getVibrator();
            if (vib.hasVibrator()) vib.vibrate(VibrationEffect.createOneShot(60, Math.max(large, small)));
        } catch (RuntimeException e) {
            // Gamepad removed in the meantime, or rumble refused by the system.
            if (!rumbleRefused[i]) Log.w("OpenXIII", "gamepad rumble refused: " + e);
            rumbleRefused[i] = true;
        }
    }

    private void stopRumble(int i) {
        Pad p = slots[i];
        InputDevice d = p == null ? null : InputDevice.getDevice(p.vibratorId);
        if (d == null) return;
        try {
            if (Build.VERSION.SDK_INT >= 31) d.getVibratorManager().cancel();
            else d.getVibrator().cancel();
        } catch (RuntimeException e) {
            // Gamepad removed.
        }
    }

    static boolean isGamepad(InputDevice d) {
        if (d == null || d.isVirtual()) return false;
        int s = d.getSources();
        return (s & InputDevice.SOURCE_GAMEPAD) == InputDevice.SOURCE_GAMEPAD
                || (s & InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK;
    }

    private static boolean fromGamepad(int source) {
        return (source & InputDevice.SOURCE_GAMEPAD) == InputDevice.SOURCE_GAMEPAD
                || (source & InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK
                || (source & InputDevice.SOURCE_DPAD) == InputDevice.SOURCE_DPAD;
    }

    /** Slot of this gamepad, taking the first free one if needed; -1 if all 4 are taken. */
    private int slot(int deviceId) {
        int i = slotOf.get(deviceId, -1);
        if (i >= 0) return i;
        for (i = 0; i < slots.length; i++) {
            if (slots[i] == null) {
                Pad p = new Pad();
                p.deviceId = deviceId;
                p.vibratorId = vibratorOf(deviceId);
                slots[i] = p;
                slotOf.put(deviceId, i);
                rumbleRefused[i] = false;
                InputDevice d = InputDevice.getDevice(deviceId);
                String info = (d != null ? d.getName() : "#" + deviceId) + " (" + rumbleInfo(p.vibratorId) + ")";
                Log.i("OpenXIII", "gamepad " + deviceId + ": " + info + (p.vibratorId != deviceId ? ", motors on device " + p.vibratorId : ""));
                toast("Gamepad connected: " + info);
                slotsChanged();
                return i;
            }
        }
        return -1;
    }

    /** Motors of a device: 0 without rumble (Android 12+: its vibrators, otherwise 1 if
     *  getVibrator().hasVibrator()). */
    private static int motors(InputDevice d) {
        if (d == null) return 0;
        try {
            if (Build.VERSION.SDK_INT >= 31) {
                int n = d.getVibratorManager().getVibratorIds().length;
                if (n > 0) return n;
            }
            return d.getVibrator().hasVibrator() ? 1 : 0;
        } catch (RuntimeException e) {
            return 0;
        }
    }

    /** The gamepad's device if it has motors; otherwise another device of the same physical
     *  gamepad that has some (a gamepad can appear as several input devices); otherwise itself. */
    private static int vibratorOf(int deviceId) {
        InputDevice d = InputDevice.getDevice(deviceId);
        if (d == null || motors(d) > 0) return deviceId;
        for (int id : InputDevice.getDeviceIds()) {
            if (id == deviceId) continue;
            InputDevice o = InputDevice.getDevice(id);
            if (o == null || o.isVirtual()) continue;
            boolean same = d.getDescriptor() != null && d.getDescriptor().equals(o.getDescriptor());
            if (!same && d.getVendorId() != 0) same = o.getVendorId() == d.getVendorId() && o.getProductId() == d.getProductId();
            if (same && motors(o) > 0) return id;
        }
        return deviceId;
    }

    /** Toast text: number of motors Android gives for the gamepad. */
    private static String rumbleInfo(int deviceId) {
        int n = motors(InputDevice.getDevice(deviceId));
        if (n == 0) return "no rumble on Android";
        return n == 1 ? "rumble: 1 motor" : "rumble: " + n + " motors";
    }

    private void send(int i) {
        Pad p = slots[i];
        try {
            if (p == null) NativeInput.pad(i, false, 0, 0, 0, 0, 0, 0, 0);
            else NativeInput.pad(i, true, p.lx, p.ly, p.rx, p.ry, p.lt, p.rt, p.keys | p.hat);
        } catch (UnsatisfiedLinkError e) {
            // Game library not loaded yet.
        }
    }

    @Override
    public void onInputDeviceAdded(int deviceId) {
        // Another device of a connected gamepad may bring its motors after it.
        for (Pad p : slots) {
            if (p != null && motors(InputDevice.getDevice(p.vibratorId)) == 0) p.vibratorId = vibratorOf(p.deviceId);
        }
        if (!isGamepad(InputDevice.getDevice(deviceId))) return;
        int i = slot(deviceId);
        if (i >= 0) send(i);
    }

    @Override
    public void onInputDeviceRemoved(int deviceId) {
        int i = slotOf.get(deviceId, -1);
        if (i < 0) return;
        slotOf.delete(deviceId);
        slots[i] = null;
        send(i);
        toast("Gamepad disconnected");
        slotsChanged();
    }

    @Override
    public void onInputDeviceChanged(int deviceId) {}

    private void toast(String s) {
        Toast.makeText(context, s, Toast.LENGTH_SHORT).show();
    }

    private static int bit(int keyCode) {
        switch (keyCode) {
            case KeyEvent.KEYCODE_BUTTON_A: return A;
            case KeyEvent.KEYCODE_BUTTON_B: return B;
            case KeyEvent.KEYCODE_BUTTON_X: return X;
            case KeyEvent.KEYCODE_BUTTON_Y: return Y;
            case KeyEvent.KEYCODE_BUTTON_L1: return LB;
            case KeyEvent.KEYCODE_BUTTON_R1: return RB;
            case KeyEvent.KEYCODE_BUTTON_SELECT: return BACK;
            case KeyEvent.KEYCODE_BUTTON_START: return START;
            case KeyEvent.KEYCODE_BUTTON_THUMBL: return THUMB_L;
            case KeyEvent.KEYCODE_BUTTON_THUMBR: return THUMB_R;
            case KeyEvent.KEYCODE_DPAD_UP: return UP;
            case KeyEvent.KEYCODE_DPAD_DOWN: return DOWN;
            case KeyEvent.KEYCODE_DPAD_LEFT: return LEFT;
            case KeyEvent.KEYCODE_DPAD_RIGHT: return RIGHT;
            // Generic (DirectInput) gamepads with no layout known to Android: numbered buttons.
            case KeyEvent.KEYCODE_BUTTON_1: return A;
            case KeyEvent.KEYCODE_BUTTON_2: return B;
            case KeyEvent.KEYCODE_BUTTON_3: return X;
            case KeyEvent.KEYCODE_BUTTON_4: return Y;
            case KeyEvent.KEYCODE_BUTTON_5: return LB;
            case KeyEvent.KEYCODE_BUTTON_6: return RB;
            case KeyEvent.KEYCODE_BUTTON_9: return BACK;
            case KeyEvent.KEYCODE_BUTTON_10: return START;
            case KeyEvent.KEYCODE_BUTTON_11: return THUMB_L;
            case KeyEvent.KEYCODE_BUTTON_12: return THUMB_R;
            default: return 0;
        }
    }

    /** A gamepad key: taken (true) or left to the rest of the activity. */
    boolean onKey(KeyEvent e) {
        if (!fromGamepad(e.getSource())) return false;
        int b = bit(e.getKeyCode());
        // Triggers reported as buttons by some gamepads.
        int code = e.getKeyCode();
        boolean left = code == KeyEvent.KEYCODE_BUTTON_L2 || code == KeyEvent.KEYCODE_BUTTON_7;
        boolean trigger = left || code == KeyEvent.KEYCODE_BUTTON_R2 || code == KeyEvent.KEYCODE_BUTTON_8;
        if (b == 0 && !trigger) return false;
        int i = slot(e.getDeviceId());
        if (i < 0) return true;
        Pad p = slots[i];
        boolean down = e.getAction() == KeyEvent.ACTION_DOWN;
        if (trigger) {
            if (left) p.lt = down ? 1f : 0f;
            else p.rt = down ? 1f : 0f;
        } else if (down) {
            p.keys |= b;
        } else if (e.getAction() == KeyEvent.ACTION_UP) {
            p.keys &= ~b;
        }
        send(i);
        return true;
    }

    /** Sticks, triggers and d-pad (HAT axes) of a gamepad. */
    boolean onMotion(MotionEvent e) {
        if (!fromGamepad(e.getSource()) || e.getAction() != MotionEvent.ACTION_MOVE) return false;
        int i = slot(e.getDeviceId());
        if (i < 0) return true;
        Pad p = slots[i];
        InputDevice d = e.getDevice();
        p.lx = axis(e, d, MotionEvent.AXIS_X);
        p.ly = axis(e, d, MotionEvent.AXIS_Y);
        p.rx = axis(e, d, MotionEvent.AXIS_Z);
        p.ry = axis(e, d, MotionEvent.AXIS_RZ);
        float lt = Math.max(axis(e, d, MotionEvent.AXIS_LTRIGGER), axis(e, d, MotionEvent.AXIS_BRAKE));
        float rt = Math.max(axis(e, d, MotionEvent.AXIS_RTRIGGER), axis(e, d, MotionEvent.AXIS_GAS));
        if (d != null && (d.getMotionRange(MotionEvent.AXIS_LTRIGGER) != null || d.getMotionRange(MotionEvent.AXIS_BRAKE) != null)) p.lt = lt;
        if (d != null && (d.getMotionRange(MotionEvent.AXIS_RTRIGGER) != null || d.getMotionRange(MotionEvent.AXIS_GAS) != null)) p.rt = rt;
        float hx = e.getAxisValue(MotionEvent.AXIS_HAT_X), hy = e.getAxisValue(MotionEvent.AXIS_HAT_Y);
        p.hat = (hx < -0.5f ? LEFT : 0) | (hx > 0.5f ? RIGHT : 0) | (hy < -0.5f ? UP : 0) | (hy > 0.5f ? DOWN : 0);
        send(i);
        return true;
    }

    /** Axis without the gamepad's dead zone (flat). */
    private static float axis(MotionEvent e, InputDevice d, int axis) {
        float v = e.getAxisValue(axis);
        InputDevice.MotionRange r = d == null ? null : d.getMotionRange(axis, e.getSource());
        float flat = r == null ? 0f : r.getFlat();
        return Math.abs(v) > flat ? v : 0f;
    }
}

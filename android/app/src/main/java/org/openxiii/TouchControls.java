package org.openxiii;

import android.app.AlertDialog;
import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.DashPathEffect;
import android.graphics.Paint;
import android.graphics.RectF;
import android.graphics.drawable.GradientDrawable;
import android.util.SparseArray;
import android.view.Gravity;
import android.view.HapticFeedbackConstants;
import android.view.MotionEvent;
import android.view.View;
import android.widget.Button;
import android.widget.CheckBox;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.SeekBar;
import android.widget.TextView;
import android.widget.Toast;

import java.util.HashMap;

/**
 * Touch overlay put on top of the game activity: sticks and buttons sent to
 * {@link NativeInput}, fingers outside the controls sent to NativeInput.touch(),
 * and an edit mode to move, resize, hide or add controls.
 *
 * <pre>
 *   TouchControls tc = new TouchControls(activity);
 *   activity.addContentView(tc, new ViewGroup.LayoutParams(MATCH_PARENT, MATCH_PARENT));
 * </pre>
 */
public final class TouchControls extends FrameLayout {
    /** Stick dead zone, as a fraction of the stick reach. */
    private static final float DEAD_ZONE = 0.12f;
    /** touch() ids of fingers aiming from a button: Android id + this. */
    public static final int AIM_ID_OFFSET = 100;
    private static final float SLOP_DP = 8f;
    /** Lower-left area where the floating move stick appears. */
    private static final float MOVE_ZONE_RIGHT = 0.45f;
    private static final float MOVE_ZONE_TOP = 0.30f;

    private static final int FREE = 0, BUTTON = 1, STICK = 2, EDIT_BUTTON = 3, EDIT_DRAG = 4, EDIT_IDLE = 5;

    private static final class Finger {
        int mode;
        TouchLayout.Control control;
        float downX, downY;
        /** Stick centre and knob offset, in pixels. */
        float cx, cy, knobX, knobY;
        float outX, outY;
        boolean aiming;
        float offX, offY;
        /** Last position sent to touch(), to skip moves that did not move. */
        float lastX, lastY;
    }

    private TouchLayout layout;
    private final float density;
    private boolean shown = true;
    private boolean editing;
    private final SparseArray<Finger> fingers = new SparseArray<>();
    /** Where this view is on the screen (from the last touch event): touch() gets screen pixels. */
    private float screenX, screenY;
    /** Run as a finger lands, before it is sent (the activity reports where the game image is). */
    private Runnable beforeDown;
    /** How many fingers hold each key or mouse button (copies share it). */
    private final HashMap<Integer, Integer> held = new HashMap<>();

    private final Paint fill = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint stroke = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint text = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint zone = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final RectF rect = new RectF();

    // Edit mode
    private TouchLayout.Control selected;
    private float pinchStartDist, pinchStartScale;
    private boolean pinching;
    private final LinearLayout panel;
    /** Edit panel: drag handle on top, then the scrolling settings. */
    private final LinearLayout panelBox;
    private final ScrollView panelScroll;
    private final TextView handleLabel;
    private final Button collapseButton;
    private final SeekBar sizeBar, opacityBar, lookBar;
    private final TextView sizeLabel, opacityLabel, lookLabel;
    private final CheckBox floatingBox, aimBox, autoAimBox, godBox, ammoBox;
    private final Button speedButton;
    private static final float HANDLE_DP = 32f, PANEL_WIDTH_DP = 380f, PANEL_MARGIN_DP = 8f;
    /** Panel place and folded state, kept for the whole session (also across activities). */
    private static boolean panelPlaced, panelCollapsed;
    private static float panelX, panelY;
    /** A gamepad is connected: only the Edit button is drawn (except in edit mode). */
    private boolean gamepadMode;
    /** refreshPanel() is setting the boxes from the game: no change to send back. */
    private boolean refreshing;
    /** Aim assist switch: when the player last switched it (uptime ms), and how often the open
     * panel re-reads the game's state. */
    private long aimToggled;
    private static final long AIM_REFRESH_MS = 500;
    private static final long AIM_SETTLE_MS = 1000;

    public TouchControls(Context context) {
        super(context);
        density = context.getResources().getDisplayMetrics().density;
        layout = TouchLayout.load(context);
        NativeInput.lookSpeed(layout.lookSpeed);
        NativeInput.setGodMode(layout.godMode);
        NativeInput.setInfiniteAmmo(layout.infiniteAmmo);
        NativeInput.setMoveSpeed(layout.moveSpeed);
        setWillNotDraw(false);
        setFocusable(false);
        setHapticFeedbackEnabled(true);

        stroke.setStyle(Paint.Style.STROKE);
        stroke.setStrokeWidth(2f * density);
        text.setTextAlign(Paint.Align.CENTER);
        text.setFakeBoldText(true);
        zone.setStyle(Paint.Style.STROKE);
        zone.setStrokeWidth(1.5f * density);
        zone.setPathEffect(new DashPathEffect(new float[] {8 * density, 6 * density}, 0));

        // --- Edit panel ---
        panel = new LinearLayout(context);
        panel.setOrientation(LinearLayout.VERTICAL);
        int pad = dp(10);
        panel.setPadding(pad, pad, pad, pad);
        GradientDrawable bg = new GradientDrawable();
        bg.setColor(0xE0202020);
        bg.setCornerRadius(dp(12));
        panel.setBackground(bg);
        panel.setClickable(true);

        LinearLayout row = new LinearLayout(context);
        row.addView(button("Done", v -> setEditing(false)));
        row.addView(button("Add", v -> chooseControlToAdd()));
        row.addView(button("Hide", v -> hideSelected()));
        row.addView(button("Reset", v -> confirmReset()));
        panel.addView(row);

        sizeLabel = label();
        panel.addView(sizeLabel);
        sizeBar = new SeekBar(context);
        sizeBar.setMax(Math.round((TouchLayout.MAX_SCALE - TouchLayout.MIN_SCALE) * 100));
        sizeBar.setOnSeekBarChangeListener(new SeekListener() {
            @Override
            public void onProgressChanged(SeekBar s, int p, boolean user) {
                if (user && selected != null) {
                    selected.scale = TouchLayout.MIN_SCALE + p / 100f;
                    refreshPanel();
                    invalidate();
                }
            }
        });
        panel.addView(sizeBar, new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(32)));

        opacityLabel = label();
        panel.addView(opacityLabel);
        opacityBar = new SeekBar(context);
        opacityBar.setMax(Math.round((TouchLayout.MAX_OPACITY - TouchLayout.MIN_OPACITY) * 100));
        opacityBar.setOnSeekBarChangeListener(new SeekListener() {
            @Override
            public void onProgressChanged(SeekBar s, int p, boolean user) {
                if (user) {
                    layout.opacity = TouchLayout.MIN_OPACITY + p / 100f;
                    refreshPanel();
                    invalidate();
                }
            }
        });
        panel.addView(opacityBar, new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(32)));

        lookLabel = label();
        panel.addView(lookLabel);
        lookBar = new SeekBar(context);
        lookBar.setMax(Math.round((TouchLayout.MAX_LOOK_SPEED - TouchLayout.MIN_LOOK_SPEED) * 100));
        lookBar.setOnSeekBarChangeListener(new SeekListener() {
            @Override
            public void onProgressChanged(SeekBar s, int p, boolean user) {
                if (user) {
                    layout.lookSpeed = TouchLayout.MIN_LOOK_SPEED + p / 100f;
                    refreshPanel();
                }
            }
        });
        panel.addView(lookBar, new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(32)));

        floatingBox = checkBox("Floating move stick (lower left)");
        floatingBox.setOnCheckedChangeListener((b, on) -> {
            layout.floatingStick = on;
            invalidate();
        });
        panel.addView(floatingBox);
        aimBox = checkBox("Sliding from a held button also aims");
        aimBox.setOnCheckedChangeListener((b, on) -> layout.buttonsAim = on);
        panel.addView(aimBox);
        autoAimBox = checkBox("Aim assist (console version)");
        autoAimBox.setOnCheckedChangeListener((b, on) -> {
            if (refreshing) return;
            aimToggled = android.os.SystemClock.uptimeMillis();
            NativeInput.setAutoAim(on);
        });
        panel.addView(autoAimBox);
        godBox = checkBox("God mode (invincible)");
        godBox.setOnCheckedChangeListener((b, on) -> {
            layout.godMode = on;
            NativeInput.setGodMode(on);
        });
        panel.addView(godBox);
        ammoBox = checkBox("Infinite ammo");
        ammoBox.setOnCheckedChangeListener((b, on) -> {
            layout.infiniteAmmo = on;
            NativeInput.setInfiniteAmmo(on);
        });
        panel.addView(ammoBox);
        speedButton = button("", v -> {
            layout.moveSpeed = layout.moveSpeed % 3 + 1;
            NativeInput.setMoveSpeed(layout.moveSpeed);
            refreshPanel();
        });
        panel.addView(speedButton);
        // Normal build (library without the "cheats" feature): no cheats in the panel (aim assist
        // is a setting, kept in both builds).
        if (!NativeInput.cheats()) {
            godBox.setVisibility(GONE);
            ammoBox.setVisibility(GONE);
            speedButton.setVisibility(GONE);
        }

        // The settings scroll when they are taller than the screen, never taller than the screen
        // minus the handle.
        panelScroll = new ScrollView(context) {
            @Override
            protected void onMeasure(int widthSpec, int heightSpec) {
                int max = TouchControls.this.getHeight() - dp(HANDLE_DP) - 2 * dp(PANEL_MARGIN_DP);
                if (max > 0) {
                    int size = MeasureSpec.getSize(heightSpec);
                    if (MeasureSpec.getMode(heightSpec) == MeasureSpec.UNSPECIFIED || size > max) {
                        heightSpec = MeasureSpec.makeMeasureSpec(max, MeasureSpec.AT_MOST);
                    }
                }
                super.onMeasure(widthSpec, heightSpec);
            }
        };
        panelScroll.addView(panel);

        // Handle: fold button, then the area dragged to move the panel. Outside the scroll view,
        // so it stays in place while the settings scroll.
        LinearLayout handle = new LinearLayout(context);
        handle.setOrientation(LinearLayout.HORIZONTAL);
        handle.setGravity(Gravity.CENTER_VERTICAL);
        GradientDrawable handleBg = new GradientDrawable();
        handleBg.setColor(0xF0404040);
        handleBg.setStroke(dp(1), 0xFFFFD040);
        handleBg.setCornerRadius(dp(8));
        handle.setBackground(handleBg);
        collapseButton = button("", v -> setPanelCollapsed(!panelCollapsed));
        collapseButton.setTextSize(16);
        handle.addView(collapseButton, new LinearLayout.LayoutParams(dp(44), LinearLayout.LayoutParams.MATCH_PARENT));
        handleLabel = label();
        handleLabel.setTextColor(0xFFFFD040);
        handleLabel.setGravity(Gravity.CENTER);
        handleLabel.setPadding(dp(8), 0, dp(12), 0);
        handleLabel.setSingleLine(true);
        handle.addView(handleLabel, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, 1f));
        handleLabel.setOnTouchListener(new OnTouchListener() {
            private float startRawX, startRawY, startX, startY;

            @Override
            public boolean onTouch(View v, MotionEvent e) {
                switch (e.getActionMasked()) {
                    case MotionEvent.ACTION_DOWN:
                        startRawX = e.getRawX();
                        startRawY = e.getRawY();
                        startX = panelBox.getTranslationX();
                        startY = panelBox.getTranslationY();
                        return true;
                    case MotionEvent.ACTION_MOVE:
                        placePanel(startX + e.getRawX() - startRawX, startY + e.getRawY() - startRawY);
                        return true;
                    default:
                        return true;
                }
            }
        });

        panelBox = new LinearLayout(context);
        panelBox.setOrientation(LinearLayout.VERTICAL);
        panelBox.addView(handle, new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(HANDLE_DP)));
        panelBox.addView(panelScroll, new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT));
        panelBox.setVisibility(GONE);
        addView(panelBox, new LayoutParams(dp(PANEL_WIDTH_DP), LayoutParams.WRAP_CONTENT, Gravity.TOP | Gravity.LEFT));
        // First placement (centred) and back inside the screen after a size change or a fold.
        panelBox.addOnLayoutChangeListener((v, l, t, r, b, ol, ot, or, ob) -> {
            if (getWidth() == 0 || getHeight() == 0) return;
            if (!panelPlaced) {
                panelPlaced = true;
                placePanel((getWidth() - v.getWidth()) / 2f, Math.max(dp(PANEL_MARGIN_DP), (getHeight() - v.getHeight()) / 2f));
            } else {
                placePanel(panelX, panelY);
            }
        });
        applyCollapsed();
    }

    // --- Public API -----------------------------------------------------------

    /** Shows or hides the controls; hidden, every finger goes to touch(). */
    public void setControlsShown(boolean show) {
        if (show == shown) return;
        releaseAll();
        if (!show && editing) setEditing(false);
        shown = show;
        invalidate();
    }

    public boolean isControlsShown() {
        return shown;
    }

    /**
     * Gamepad connected: sticks and buttons are no longer drawn and fingers go to touch(), but the
     * Edit button stays (edit mode still shows every control so they can be set up).
     */
    public void setGamepadMode(boolean on) {
        if (on == gamepadMode) return;
        if (!editing) releaseAll();
        gamepadMode = on;
        invalidate();
    }

    public boolean isGamepadMode() {
        return gamepadMode;
    }

    public boolean isEditing() {
        return editing;
    }

    public void setEditing(boolean edit) {
        if (edit == editing) return;
        releaseAll();
        editing = edit;
        selected = null;
        pinching = false;
        if (!edit) layout.save(getContext());
        panelBox.setVisibility(edit ? VISIBLE : GONE);
        panelBox.setAlpha(1f);
        refreshPanel();
        // Aim assist as the game has it, read on entering edit mode and again while the panel is
        // open (the player may not exist yet when it opens, during a load).
        aimToggled = 0;
        refreshAutoAim();
        if (edit) postDelayed(aimRefresh, AIM_REFRESH_MS);
        invalidate();
    }

    /** The aim assist switch shows the player's state (NativeInput.autoAim: 2, no player yet). */
    private void refreshAutoAim() {
        int autoAim = NativeInput.autoAim();
        autoAimBox.setEnabled(autoAim != 2);
        if (autoAim == 2 || autoAimBox.isChecked() == (autoAim == 1)) return;
        refreshing = true;
        autoAimBox.setChecked(autoAim == 1);
        refreshing = false;
    }

    /** Re-reads the aim assist while the panel is open, except just after the player switched it
     * (the game takes the change on its next frame). */
    private final Runnable aimRefresh = new Runnable() {
        @Override
        public void run() {
            if (!editing) return;
            if (android.os.SystemClock.uptimeMillis() - aimToggled > AIM_SETTLE_MS) refreshAutoAim();
            postDelayed(this, AIM_REFRESH_MS);
        }
    };

    public TouchLayout getLayout() {
        return layout;
    }

    /** Lets go of every key, button, stick and finger the overlay holds. */
    public void releaseAll() {
        for (int i = 0; i < fingers.size(); i++) {
            int id = fingers.keyAt(i);
            Finger f = fingers.valueAt(i);
            switch (f.mode) {
                case FREE:
                    sendTouch(2, id, f.downX, f.downY);
                    break;
                case BUTTON:
                    if (f.aiming) sendTouch(2, id + AIM_ID_OFFSET, f.downX, f.downY);
                    break;
                case STICK:
                    NativeInput.stick(f.control.kind.code, 0f, 0f);
                    break;
            }
        }
        fingers.clear();
        for (HashMap.Entry<Integer, Integer> e : held.entrySet()) {
            if (e.getValue() > 0) sendHeld(e.getKey(), false);
        }
        held.clear();
        invalidate();
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (!hasFocus) releaseAll();
    }

    @Override
    protected void onDetachedFromWindow() {
        releaseAll();
        if (editing) layout.save(getContext());
        super.onDetachedFromWindow();
    }

    // --- Input ----------------------------------------------------------------

    @Override
    public boolean onTouchEvent(MotionEvent e) {
        int action = e.getActionMasked();
        // Screen pixels for touch(), as the game image's place (NativeInput.viewRect): the view can
        // be laid out elsewhere for a moment at launch.
        screenX = e.getRawX() - e.getX();
        screenY = e.getRawY() - e.getY();
        switch (action) {
            case MotionEvent.ACTION_DOWN:
            case MotionEvent.ACTION_POINTER_DOWN: {
                int i = e.getActionIndex();
                if (beforeDown != null) beforeDown.run();
                if (editing) editDown(e, i);
                else down(e.getPointerId(i), e.getX(i), e.getY(i));
                break;
            }
            case MotionEvent.ACTION_MOVE:
                if (editing) editMove(e);
                else {
                    for (int i = 0; i < e.getPointerCount(); i++) move(e.getPointerId(i), e.getX(i), e.getY(i));
                }
                break;
            case MotionEvent.ACTION_UP:
            case MotionEvent.ACTION_POINTER_UP: {
                int i = e.getActionIndex();
                if (editing) editUp(e.getPointerId(i), e.getX(i), e.getY(i));
                else up(e.getPointerId(i), e.getX(i), e.getY(i));
                break;
            }
            case MotionEvent.ACTION_CANCEL:
                if (editing) {
                    fingers.clear();
                    pinching = false;
                    panelBox.setAlpha(1f);
                } else releaseAll();
                break;
        }
        return true;
    }

    /** A finger outside the controls (or every finger while they are hidden), in screen pixels. */
    private void sendTouch(int action, int id, float x, float y) {
        NativeInput.touch(action, id, x + screenX, y + screenY);
    }

    /** The activity's hook run as a finger lands. */
    public void setBeforeDown(Runnable r) {
        beforeDown = r;
    }

    private void down(int id, float x, float y) {
        // Menu or game as the game is now, not as last polled (GameActivityXiii, every 200 ms): a
        // finger put down as the first menu appears went to the hidden game controls and never
        // reached the menu.
        if (!editing) {
            try {
                boolean captured = NativeInput.captured();
                if (captured != shown) setControlsShown(captured);
            } catch (UnsatisfiedLinkError e) {
                // No game library.
            }
        }
        Finger f = new Finger();
        f.downX = f.lastX = x;
        f.downY = f.lastY = y;
        fingers.put(id, f);
        if (!shown) {
            f.mode = FREE;
            sendTouch(0, id, x, y);
            return;
        }
        if (onEditButton(x, y)) {
            f.mode = EDIT_BUTTON;
            return;
        }
        if (gamepadMode) {
            f.mode = FREE;
            sendTouch(0, id, x, y);
            return;
        }
        TouchLayout.Control c = hit(x, y);
        if (c != null && c.kind.isStick() && !stickHeld(c.kind)) {
            startStick(f, c, x, y);
            return;
        }
        if (c != null && !c.kind.isStick()) {
            f.mode = BUTTON;
            f.control = c;
            press(c.kind, true);
            performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY);
            invalidate();
            return;
        }
        TouchLayout.Control move = layout.find(TouchLayout.Kind.MOVE_STICK);
        if (c == null && move != null && layout.floatingStick && !stickHeld(move.kind)
                && x < getWidth() * MOVE_ZONE_RIGHT && y > getHeight() * MOVE_ZONE_TOP) {
            startStick(f, move, x, y);
            return;
        }
        f.mode = FREE;
        sendTouch(0, id, x, y);
    }

    private void move(int id, float x, float y) {
        Finger f = fingers.get(id);
        if (f == null) return;
        if ((f.mode == FREE || f.aiming) && x == f.lastX && y == f.lastY) return;
        switch (f.mode) {
            case FREE:
                sendTouch(1, id, x, y);
                f.lastX = x;
                f.lastY = y;
                break;
            case STICK:
                updateStick(f, x, y);
                break;
            case BUTTON:
                if (!layout.buttonsAim) break;
                if (!f.aiming && Math.hypot(x - f.downX, y - f.downY) > SLOP_DP * density) {
                    f.aiming = true;
                    sendTouch(0, id + AIM_ID_OFFSET, f.downX, f.downY);
                }
                if (f.aiming) {
                    sendTouch(1, id + AIM_ID_OFFSET, x, y);
                    f.lastX = x;
                    f.lastY = y;
                }
                break;
        }
    }

    private void up(int id, float x, float y) {
        Finger f = fingers.get(id);
        if (f == null) return;
        fingers.remove(id);
        switch (f.mode) {
            case FREE:
                sendTouch(2, id, x, y);
                break;
            case STICK:
                NativeInput.stick(f.control.kind.code, 0f, 0f);
                invalidate();
                break;
            case BUTTON:
                if (f.aiming) sendTouch(2, id + AIM_ID_OFFSET, x, y);
                press(f.control.kind, false);
                invalidate();
                break;
            case EDIT_BUTTON:
                if (onEditButton(x, y)) setEditing(true);
                break;
        }
    }

    private void startStick(Finger f, TouchLayout.Control c, float x, float y) {
        f.mode = STICK;
        f.control = c;
        boolean floating = c.kind == TouchLayout.Kind.MOVE_STICK && layout.floatingStick;
        f.cx = floating ? x : c.x * getWidth();
        f.cy = floating ? y : c.y * getHeight();
        updateStick(f, x, y);
        invalidate();
    }

    private void updateStick(Finger f, float x, float y) {
        float reach = f.control.diameter(density) / 2f;
        float dx = x - f.cx, dy = y - f.cy;
        float len = (float) Math.hypot(dx, dy);
        if (len > reach) {
            if (f.control.kind == TouchLayout.Kind.MOVE_STICK && layout.floatingStick) {
                // The floating stick follows the thumb past its edge.
                f.cx += dx * (1f - reach / len);
                f.cy += dy * (1f - reach / len);
            }
            dx *= reach / len;
            dy *= reach / len;
            len = reach;
        }
        f.knobX = dx;
        f.knobY = dy;
        float m = len / reach;
        float out = m <= DEAD_ZONE ? 0f : (m - DEAD_ZONE) / (1f - DEAD_ZONE);
        float ox = len > 0f ? dx / len * out : 0f;
        float oy = len > 0f ? dy / len * out : 0f;
        if (Math.abs(ox - f.outX) > 0.002f || Math.abs(oy - f.outY) > 0.002f || (out == 0f && (f.outX != 0f || f.outY != 0f))) {
            f.outX = ox;
            f.outY = oy;
            NativeInput.stick(f.control.kind.code, ox, oy);
        }
        invalidate();
    }

    private boolean stickHeld(TouchLayout.Kind kind) {
        for (int i = 0; i < fingers.size(); i++) {
            Finger f = fingers.valueAt(i);
            if (f.mode == STICK && f.control.kind == kind) return true;
        }
        return false;
    }

    private void press(TouchLayout.Kind k, boolean down) {
        if (k.type == TouchLayout.Type.WHEEL) {
            if (down) NativeInput.wheel(k.code);
            return;
        }
        int key = (k.type == TouchLayout.Type.MOUSE ? 0x10000 : 0) | k.code;
        Integer n = held.get(key);
        int count = n == null ? 0 : n;
        if (down) {
            if (++count == 1) sendHeld(key, true);
        } else if (count > 0) {
            if (--count == 0) sendHeld(key, false);
        }
        held.put(key, count);
    }

    private static void sendHeld(int key, boolean down) {
        if ((key & 0x10000) != 0) NativeInput.mouseButton(key & 0xFFFF, down);
        else NativeInput.key(key, down);
    }

    private boolean isPressed(TouchLayout.Control c) {
        for (int i = 0; i < fingers.size(); i++) {
            Finger f = fingers.valueAt(i);
            if (f.control == c && (f.mode == BUTTON || f.mode == STICK)) return true;
        }
        return false;
    }

    /** Topmost control under (x, y); buttons get a little extra margin. */
    private TouchLayout.Control hit(float x, float y) {
        int w = getWidth(), h = getHeight();
        for (int i = layout.controls.size() - 1; i >= 0; i--) {
            TouchLayout.Control c = layout.controls.get(i);
            float r = c.diameter(density) / 2f * (c.kind.isStick() ? 1f : 1.15f);
            if (Math.hypot(x - c.x * w, y - c.y * h) <= r) return c;
        }
        return null;
    }

    // --- Edit button (always shown with the controls) -------------------------

    private void editButtonRect(RectF r) {
        float cx = getWidth() / 2f, top = dp(8);
        r.set(cx - dp(30), top, cx + dp(30), top + dp(26));
    }

    private boolean onEditButton(float x, float y) {
        editButtonRect(rect);
        rect.inset(-dp(8), -dp(8));
        return rect.contains(x, y);
    }

    // --- Edit mode ------------------------------------------------------------

    private void editDown(MotionEvent e, int i) {
        int id = e.getPointerId(i);
        float x = e.getX(i), y = e.getY(i);
        Finger f = new Finger();
        f.downX = x;
        f.downY = y;
        fingers.put(id, f);
        if (fingers.size() == 2 && selected != null) {
            // Second finger: pinch to resize the selected control.
            for (int k = 0; k < fingers.size(); k++) fingers.valueAt(k).mode = EDIT_IDLE;
            pinching = true;
            pinchStartDist = Math.max(pinchDistance(e), dp(10));
            pinchStartScale = selected.scale;
            return;
        }
        if (fingers.size() > 1) {
            f.mode = EDIT_IDLE;
            return;
        }
        TouchLayout.Control c = hit(x, y);
        selected = c;
        if (c != null) {
            f.mode = EDIT_DRAG;
            f.control = c;
            f.offX = c.x * getWidth() - x;
            f.offY = c.y * getHeight() - y;
            // Bring it on top so it stays easy to grab.
            layout.controls.remove(c);
            layout.controls.add(c);
            // See through the panel while a control is carried over it.
            panelBox.setAlpha(0.25f);
        } else {
            f.mode = EDIT_IDLE;
        }
        refreshPanel();
        invalidate();
    }

    private void editMove(MotionEvent e) {
        if (pinching) {
            if (selected != null && e.getPointerCount() >= 2) {
                float s = pinchStartScale * pinchDistance(e) / pinchStartDist;
                selected.scale = TouchLayout.clamp(s, TouchLayout.MIN_SCALE, TouchLayout.MAX_SCALE);
                refreshPanel();
                invalidate();
            }
            return;
        }
        for (int i = 0; i < e.getPointerCount(); i++) {
            Finger f = fingers.get(e.getPointerId(i));
            if (f == null || f.mode != EDIT_DRAG) continue;
            f.control.x = TouchLayout.clamp((e.getX(i) + f.offX) / getWidth(), 0f, 1f);
            f.control.y = TouchLayout.clamp((e.getY(i) + f.offY) / getHeight(), 0f, 1f);
            invalidate();
        }
    }

    private void editUp(int id, float x, float y) {
        fingers.remove(id);
        if (fingers.size() < 2) pinching = false;
        if (fingers.size() == 0) panelBox.setAlpha(1f);
    }

    private float pinchDistance(MotionEvent e) {
        int a = -1, b = -1;
        for (int i = 0; i < e.getPointerCount(); i++) {
            if (fingers.get(e.getPointerId(i)) == null) continue;
            if (a < 0) a = i;
            else if (b < 0) b = i;
        }
        if (a < 0 || b < 0) return pinchStartDist;
        return (float) Math.hypot(e.getX(a) - e.getX(b), e.getY(a) - e.getY(b));
    }

    private void chooseControlToAdd() {
        final TouchLayout.Kind[] kinds = TouchLayout.Kind.values();
        String[] names = new String[kinds.length];
        for (int i = 0; i < kinds.length; i++) {
            int count = 0;
            for (TouchLayout.Control c : layout.controls) if (c.kind == kinds[i]) count++;
            names[i] = kinds[i].label + (count > 0 ? "  (" + count + " shown)" : "");
        }
        new AlertDialog.Builder(getContext())
                .setTitle("Add a control")
                .setItems(names, (d, which) -> {
                    TouchLayout.Kind k = kinds[which];
                    TouchLayout.Control c = layout.add(k);
                    if (c == null) {
                        toast(k.isStick() ? "This stick already shows." : "At most 64 controls. Reset removes the copies.");
                        return;
                    }
                    selected = c;
                    refreshPanel();
                    invalidate();
                })
                .setNegativeButton("Cancel", null)
                .show();
    }

    private void hideSelected() {
        if (selected == null) {
            toast("Tap a control first.");
            return;
        }
        layout.controls.remove(selected);
        selected = null;
        refreshPanel();
        invalidate();
    }

    private void confirmReset() {
        new AlertDialog.Builder(getContext())
                .setTitle("Reset the controls?")
                .setMessage("Positions, sizes, opacity and options go back to the defaults.")
                .setPositiveButton("Reset", (d, w) -> {
                    layout = TouchLayout.defaults();
                    selected = null;
                    refreshPanel();
                    invalidate();
                })
                .setNegativeButton("Cancel", null)
                .show();
    }

    /** Moves the panel to (x, y), kept so that its handle stays on the screen. */
    private void placePanel(float x, float y) {
        int w = getWidth(), h = getHeight();
        if (w > 0 && h > 0) {
            float boxW = panelBox.getWidth();
            // At least the fold button and part of the handle stay visible horizontally,
            // and the whole handle height vertically.
            float keep = Math.min(boxW, dp(120));
            x = TouchLayout.clamp(x, keep - boxW, w - keep);
            y = TouchLayout.clamp(y, 0f, Math.max(0f, h - dp(HANDLE_DP)));
        }
        panelX = x;
        panelY = y;
        panelBox.setTranslationX(x);
        panelBox.setTranslationY(y);
    }

    private void setPanelCollapsed(boolean collapsed) {
        panelCollapsed = collapsed;
        applyCollapsed();
    }

    /** Folded: only a small handle stays, the whole screen behind it is free for the controls. */
    private void applyCollapsed() {
        panelScroll.setVisibility(panelCollapsed ? GONE : VISIBLE);
        collapseButton.setText(panelCollapsed ? "+" : "–");
        handleLabel.setText(panelCollapsed ? "≡ Drag" : "≡ Drag to move");
        LinearLayout.LayoutParams lp = (LinearLayout.LayoutParams) handleLabel.getLayoutParams();
        lp.width = panelCollapsed ? LinearLayout.LayoutParams.WRAP_CONTENT : 0;
        lp.weight = panelCollapsed ? 0f : 1f;
        handleLabel.setLayoutParams(lp);
        LayoutParams box = (LayoutParams) panelBox.getLayoutParams();
        int full = dp(PANEL_WIDTH_DP);
        if (getWidth() > 0) full = Math.min(full, getWidth());
        box.width = panelCollapsed ? LayoutParams.WRAP_CONTENT : full;
        panelBox.setLayoutParams(box);
    }

    @Override
    protected void onSizeChanged(int w, int h, int oldw, int oldh) {
        super.onSizeChanged(w, h, oldw, oldh);
        // New screen size (rotation): panel width and height limit again, placed back inside.
        post(() -> {
            applyCollapsed();
            panelScroll.requestLayout();
        });
    }

    private void refreshPanel() {
        if (selected != null) {
            sizeLabel.setText("Size of " + selected.kind.label + ": " + Math.round(selected.scale * 100) + "%  (or pinch)");
            sizeBar.setEnabled(true);
            sizeBar.setProgress(Math.round((selected.scale - TouchLayout.MIN_SCALE) * 100));
        } else {
            sizeLabel.setText("Tap a control to select it, drag to move, pinch to resize");
            sizeBar.setEnabled(false);
        }
        opacityLabel.setText("Opacity: " + Math.round(layout.opacity * 100) + "%");
        opacityBar.setProgress(Math.round((layout.opacity - TouchLayout.MIN_OPACITY) * 100));
        lookLabel.setText("Camera speed (finger on the screen): " + Math.round(layout.lookSpeed * 100) + "%");
        lookBar.setProgress(Math.round((layout.lookSpeed - TouchLayout.MIN_LOOK_SPEED) * 100));
        NativeInput.lookSpeed(layout.lookSpeed);
        floatingBox.setChecked(layout.floatingStick);
        aimBox.setChecked(layout.buttonsAim);
        godBox.setChecked(layout.godMode);
        ammoBox.setChecked(layout.infiniteAmmo);
        speedButton.setText("Move speed: x" + layout.moveSpeed + "  (tap to change)");
        NativeInput.setMoveSpeed(layout.moveSpeed);
    }

    // --- Drawing --------------------------------------------------------------

    @Override
    protected void onDraw(Canvas canvas) {
        super.onDraw(canvas);
        if (!shown) return;
        int w = getWidth(), h = getHeight();
        float alpha = editing ? Math.max(layout.opacity, 0.7f) : layout.opacity;
        if (gamepadMode && !editing) {
            drawEditButton(canvas, alpha);
            return;
        }

        TouchLayout.Control move = layout.find(TouchLayout.Kind.MOVE_STICK);
        if (editing && move != null && layout.floatingStick) {
            zone.setColor(withAlpha(Color.WHITE, 0.35f));
            canvas.drawRect(0, h * MOVE_ZONE_TOP, w * MOVE_ZONE_RIGHT, h, zone);
            text.setTextSize(dp(12));
            text.setColor(withAlpha(Color.WHITE, 0.5f));
            canvas.drawText("Move stick area", w * MOVE_ZONE_RIGHT / 2f, h * MOVE_ZONE_TOP + dp(18), text);
        }

        for (TouchLayout.Control c : layout.controls) {
            float r = c.diameter(density) / 2f;
            float x = c.x * w, y = c.y * h;
            if (c.kind.isStick()) {
                Finger f = stickFinger(c);
                if (f != null && !editing) {
                    x = f.cx;
                    y = f.cy;
                }
                drawStick(canvas, c, x, y, r, f, alpha);
            } else {
                boolean pressed = !editing && isPressed(c);
                fill.setColor(pressed ? withAlpha(Color.WHITE, alpha * 0.5f) : withAlpha(Color.BLACK, alpha * 0.45f));
                canvas.drawCircle(x, y, r, fill);
                stroke.setColor(withAlpha(Color.WHITE, alpha));
                canvas.drawCircle(x, y, r, stroke);
                drawLabel(canvas, c.kind.label, x, y, r, alpha);
            }
            if (editing && c == selected) {
                stroke.setColor(0xFFFFD040);
                stroke.setStrokeWidth(3f * density);
                canvas.drawCircle(x, y, r + dp(4), stroke);
                stroke.setStrokeWidth(2f * density);
            }
        }

        if (!editing) drawEditButton(canvas, alpha);
    }

    private void drawEditButton(Canvas canvas, float alpha) {
        editButtonRect(rect);
        fill.setColor(withAlpha(Color.BLACK, alpha * 0.45f));
        canvas.drawRoundRect(rect, dp(13), dp(13), fill);
        stroke.setColor(withAlpha(Color.WHITE, alpha));
        canvas.drawRoundRect(rect, dp(13), dp(13), stroke);
        text.setTextSize(dp(12));
        text.setColor(withAlpha(Color.WHITE, alpha));
        canvas.drawText("Edit", rect.centerX(), rect.centerY() - (text.ascent() + text.descent()) / 2f, text);
    }

    private void drawStick(Canvas canvas, TouchLayout.Control c, float x, float y, float r, Finger f, float alpha) {
        boolean idleFloating = c.kind == TouchLayout.Kind.MOVE_STICK && layout.floatingStick && f == null && !editing;
        float a = idleFloating ? alpha * 0.5f : alpha;
        fill.setColor(withAlpha(Color.BLACK, a * 0.3f));
        canvas.drawCircle(x, y, r, fill);
        stroke.setColor(withAlpha(Color.WHITE, a * 0.8f));
        canvas.drawCircle(x, y, r, stroke);
        float kx = x, ky = y;
        if (f != null && !editing) {
            kx += f.knobX;
            ky += f.knobY;
        }
        fill.setColor(withAlpha(Color.WHITE, a * (f != null ? 0.6f : 0.35f)));
        canvas.drawCircle(kx, ky, r * 0.4f, fill);
        if (f == null || editing) drawLabel(canvas, c.kind.label, x, y + r * 0.62f, r * 0.55f, a);
    }

    private void drawLabel(Canvas canvas, String s, float x, float y, float r, float alpha) {
        float size = r * 0.45f;
        text.setTextSize(size);
        float tw = text.measureText(s);
        if (tw > r * 1.7f) text.setTextSize(size * r * 1.7f / tw);
        text.setColor(withAlpha(Color.WHITE, Math.min(1f, alpha + 0.2f)));
        canvas.drawText(s, x, y - (text.ascent() + text.descent()) / 2f, text);
    }

    private Finger stickFinger(TouchLayout.Control c) {
        for (int i = 0; i < fingers.size(); i++) {
            Finger f = fingers.valueAt(i);
            if (f.mode == STICK && f.control == c) return f;
        }
        return null;
    }

    // --- Small helpers --------------------------------------------------------

    private static int withAlpha(int color, float a) {
        return (Math.round(TouchLayout.clamp(a, 0f, 1f) * 255) << 24) | (color & 0xFFFFFF);
    }

    private int dp(float v) {
        return Math.round(v * density);
    }

    private Button button(String s, View.OnClickListener l) {
        Button b = new Button(getContext());
        b.setText(s);
        b.setAllCaps(false);
        b.setTextSize(13);
        b.setMinWidth(0);
        b.setMinimumWidth(0);
        b.setPadding(dp(8), 0, dp(8), 0);
        b.setOnClickListener(l);
        return b;
    }

    private TextView label() {
        TextView t = new TextView(getContext());
        t.setTextColor(Color.WHITE);
        t.setTextSize(13);
        return t;
    }

    private CheckBox checkBox(String s) {
        CheckBox b = new CheckBox(getContext());
        b.setText(s);
        b.setTextColor(Color.WHITE);
        b.setTextSize(13);
        return b;
    }

    private void toast(String s) {
        Toast.makeText(getContext(), s, Toast.LENGTH_SHORT).show();
    }

    private abstract static class SeekListener implements SeekBar.OnSeekBarChangeListener {
        @Override
        public void onStartTrackingTouch(SeekBar s) {}

        @Override
        public void onStopTrackingTouch(SeekBar s) {}
    }
}

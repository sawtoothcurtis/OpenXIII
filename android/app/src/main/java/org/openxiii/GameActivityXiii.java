package org.openxiii;

import android.content.Context;
import android.graphics.Color;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowManager;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.TextView;

import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.core.view.WindowInsetsControllerCompat;

import com.google.androidgamesdk.GameActivity;

/**
 * The game: GameActivity loads libopenxiii.so (android_main, crates/openxiii-android) and gives it
 * its surface; the touch controls are laid over it.
 */
public class GameActivityXiii extends GameActivity {
    private TouchControls controls;
    /** Phone's keyboard for the menus' text fields. */
    private TextKeyboard keyboard;
    private Gamepads gamepads;
    /** Shown over the game until its first image: a turning wheel and, while the menus'
     * pictures are made, their percent (a first launch can take a minute or more). */
    private View loadingScreen;
    private TextView loadingPercent;
    /** Wi-Fi UDP broadcasts received while playing (local network game search). */
    private WifiManager.MulticastLock multicast;
    /** Lines of the game image (see onCreate). */
    private static final int RENDER_HEIGHT = 600;
    /** The screen in landscape (pixels). */
    private int screenW, screenH;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        if (Build.VERSION.SDK_INT >= 28) {
            getWindow().getAttributes().layoutInDisplayCutoutMode =
                    WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
        }
        hideSystemBars();
        // Black around a game image narrower than the screen (a 4:3 resolution).
        getWindow().getDecorView().setBackgroundColor(Color.BLACK);
        // Game image of RENDER_HEIGHT lines, scaled up to the screen by the phone (like a monitor
        // showing XIII.ini's 800 × 600 in fullscreen): menus at their intended size and less work.
        // Same aspect ratio as the screen. The game then asks for the resolution of its ini or of
        // its Video menu (watchCapture, applySurface).
        android.util.DisplayMetrics dm = new android.util.DisplayMetrics();
        getWindowManager().getDefaultDisplay().getRealMetrics(dm);
        screenW = Math.max(dm.widthPixels, dm.heightPixels);
        screenH = Math.min(dm.widthPixels, dm.heightPixels);
        try {
            NativeInput.screenSize(screenW, screenH);
        } catch (UnsatisfiedLinkError e) {
            // No game library: the image stays as below.
        }
        if (screenH > RENDER_HEIGHT) {
            mSurfaceView.getHolder().setFixedSize(Math.round(screenW * (float) RENDER_HEIGHT / screenH), RENDER_HEIGHT);
        }
        // Where the game image is on the screen, for the fingers (screen pixels, as TouchControls
        // sends them): after each layout, as a finger lands and at each poll.
        mSurfaceView.addOnLayoutChangeListener((v, l, t, r, b, ol, ot, or, ob) -> reportViewRect());
        controls = new TouchControls(this);
        controls.setBeforeDown(this::reportViewRect);
        addContentView(controls, new ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        controls.setControlsShown(false);
        loadingScreen = makeLoadingScreen();
        addContentView(loadingScreen, new ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        keyboard = new TextKeyboard(this);
        addContentView(keyboard, new ViewGroup.LayoutParams(1, 1));
        handler.post(watchCapture);
        gamepads = new Gamepads(this);
        // With a gamepad, only the Edit button of the touch controls stays on the screen.
        gamepads.setListener(any -> controls.setGamepadMode(any));
        try {
            WifiManager wifi = (WifiManager) getApplicationContext().getSystemService(Context.WIFI_SERVICE);
            if (wifi != null) {
                multicast = wifi.createMulticastLock("openxiii-lan");
                multicast.setReferenceCounted(false);
                multicast.acquire();
            }
        } catch (RuntimeException e) {
            multicast = null;
        }
    }

    /** Black screen with a turning wheel in the middle and the percent below it (no words:
     * the same in every language). */
    private View makeLoadingScreen() {
        LinearLayout box = new LinearLayout(this);
        box.setOrientation(LinearLayout.VERTICAL);
        box.setGravity(Gravity.CENTER);
        box.setBackgroundColor(Color.BLACK);
        // Touches while loading go nowhere.
        box.setClickable(true);
        ProgressBar wheel = new ProgressBar(this, null, android.R.attr.progressBarStyleLarge);
        wheel.setIndeterminate(true);
        box.addView(wheel);
        loadingPercent = new TextView(this);
        loadingPercent.setTextColor(Color.WHITE);
        loadingPercent.setTextSize(android.util.TypedValue.COMPLEX_UNIT_SP, 20);
        loadingPercent.setGravity(Gravity.CENTER);
        loadingPercent.setPadding(0, Math.round(16 * getResources().getDisplayMetrics().density), 0, 0);
        box.addView(loadingPercent);
        return box;
    }

    /** The loading screen follows the game's startup (NativeInput.loading). */
    private void updateLoadingScreen() {
        if (loadingScreen == null || loadingScreen.getVisibility() == View.GONE) return;
        int state;
        try {
            state = NativeInput.loading();
        } catch (UnsatisfiedLinkError e) {
            state = -1;
        }
        if (state == -1) {
            loadingScreen.setVisibility(View.GONE);
        } else if (state >= 0) {
            loadingPercent.setText(state + " %");
        }
    }

    private final int[] viewOnScreen = new int[2];

    /** Tells the game where its image is on the screen (NativeInput.viewRect, screen pixels). */
    private void reportViewRect() {
        int w = mSurfaceView.getWidth(), h = mSurfaceView.getHeight();
        if (w <= 0 || h <= 0) return;
        mSurfaceView.getLocationOnScreen(viewOnScreen);
        try {
            NativeInput.viewRect(viewOnScreen[0], viewOnScreen[1], w, h);
        } catch (UnsatisfiedLinkError e) {
            // No game library.
        }
    }

    /** Controls shown while playing (mouse captured); in the menus, the finger is the cursor. */
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final Runnable watchCapture = new Runnable() {
        @Override
        public void run() {
            boolean captured;
            try {
                captured = NativeInput.captured();
            } catch (UnsatisfiedLinkError e) {
                captured = false;
            }
            if (!controls.isEditing()) controls.setControlsShown(captured);
            reportViewRect();
            long wanted;
            try {
                wanted = NativeInput.wantedSurface();
            } catch (UnsatisfiedLinkError e) {
                wanted = 0;
            }
            if (wanted != 0) applySurface((int) (wanted >>> 32), (int) (wanted & 0xFFFFFFFFL));
            keyboard.poll();
            updateLoadingScreen();
            handler.postDelayed(this, 200);
        }
    };

    /**
     * The game image becomes w × h pixels (a resolution of the game's Video menu), shown as a
     * monitor keeping the aspect ratio would: as large as the screen allows, centred, black
     * around it (a 4:3 image on a wide phone has bars on its left and right).
     */
    private void applySurface(int w, int h) {
        if (w <= 0 || h <= 0) return;
        mSurfaceView.getHolder().setFixedSize(w, h);
        View parent = (View) mSurfaceView.getParent();
        int sw = parent != null && parent.getWidth() > 0 ? parent.getWidth() : screenW;
        int sh = parent != null && parent.getHeight() > 0 ? parent.getHeight() : screenH;
        int vw = sw, vh = sh;
        // A size in the screen's own shape, rounded to whole pixels, fills the screen.
        float image = w / (float) h, screen = sw / (float) sh;
        if (image < screen * 0.99f) {
            vw = Math.round(sh * image);
        } else if (image > screen * 1.01f) {
            vh = Math.round(sw / image);
        }
        ViewGroup.LayoutParams lp = mSurfaceView.getLayoutParams();
        if (lp instanceof FrameLayout.LayoutParams) {
            FrameLayout.LayoutParams f = (FrameLayout.LayoutParams) lp;
            f.width = vw == sw ? ViewGroup.LayoutParams.MATCH_PARENT : vw;
            f.height = vh == sh ? ViewGroup.LayoutParams.MATCH_PARENT : vh;
            f.gravity = Gravity.CENTER;
            mSurfaceView.setLayoutParams(f);
        } else {
            mSurfaceView.setLayoutParams(new FrameLayout.LayoutParams(
                    vw == sw ? ViewGroup.LayoutParams.MATCH_PARENT : vw,
                    vh == sh ? ViewGroup.LayoutParams.MATCH_PARENT : vh, Gravity.CENTER));
        }
    }

    @Override
    protected void onDestroy() {
        handler.removeCallbacks(watchCapture);
        if (gamepads != null) gamepads.close();
        if (multicast != null && multicast.isHeld()) multicast.release();
        super.onDestroy();
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            hideSystemBars();
        } else if (controls != null) {
            controls.releaseAll();
        }
    }

    /** Android Back: Escape, which opens the game menu or goes back in the menus. */
    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        if (gamepads != null && gamepads.onKey(event)) return true;
        if (event.getKeyCode() == KeyEvent.KEYCODE_BACK) {
            if (event.getAction() == KeyEvent.ACTION_DOWN && event.getRepeatCount() == 0) {
                NativeInput.key(0x1B, true);
            } else if (event.getAction() == KeyEvent.ACTION_UP) {
                NativeInput.key(0x1B, false);
            }
            return true;
        }
        return super.dispatchKeyEvent(event);
    }

    /** A tap on a menu text field brings back the keyboard closed by the player. */
    @Override
    public boolean dispatchTouchEvent(MotionEvent event) {
        if (keyboard != null && event.getActionMasked() == MotionEvent.ACTION_UP) keyboard.onTap();
        return super.dispatchTouchEvent(event);
    }

    /** Sticks and triggers of a gamepad. */
    @Override
    public boolean dispatchGenericMotionEvent(MotionEvent event) {
        if (gamepads != null && gamepads.onMotion(event)) return true;
        return super.dispatchGenericMotionEvent(event);
    }

    private void hideSystemBars() {
        View decor = getWindow().getDecorView();
        WindowCompat.setDecorFitsSystemWindows(getWindow(), false);
        WindowInsetsControllerCompat c = WindowCompat.getInsetsController(getWindow(), decor);
        c.hide(WindowInsetsCompat.Type.systemBars());
        c.setSystemBarsBehavior(WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
    }
}

package org.openxiii;

import android.Manifest;
import android.app.Activity;
import android.app.ActivityManager;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.graphics.Typeface;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.provider.DocumentsContract;
import android.provider.Settings;
import android.util.TypedValue;
import android.view.Gravity;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;

/**
 * Launcher, like OpenXIII's XIII.exe on PC (launcher.rs): on the first start, the player points to
 * the original game's folder on the phone; it is remembered and read in place (never modified).
 * Afterwards the game starts directly. The icon's "Game folder" shortcut chooses it again.
 */
public class LauncherActivity extends Activity {
    static final String BRAND = "OpenXIII 0.1";
    private static final String ACTION_CHANGE = "org.openxiii.CHANGE_FOLDER";
    private static final int PICK_FOLDER = 1;
    private static final int ASK_STORAGE = 2;
    private static final int PICK_EXE = 3;

    private SharedPreferences prefs;
    private TextView status;
    private Button access;
    private Button play;
    private Button updates;
    private boolean changing;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        prefs = getSharedPreferences("openxiii", MODE_PRIVATE);
        changing = ACTION_CHANGE.equals(getIntent().getAction());
        if (changing) {
            stopGame();
        } else if (gameDir() != null && hasAccess()) {
            // A newer official version is announced first (UpdateNotice.java), then the game starts.
            UpdateNotice.atLaunch(this, prefs, this::launch);
            return;
        }
        buildUi();
    }

    @Override
    protected void onResume() {
        super.onResume();
        if (status != null) refresh();
    }

    // ----- Interface -----

    private void buildUi() {
        int pad = dp(24);
        LinearLayout box = new LinearLayout(this);
        box.setOrientation(LinearLayout.VERTICAL);
        box.setPadding(pad, pad, pad, pad);
        box.setGravity(Gravity.CENTER_HORIZONTAL);

        TextView title = new TextView(this);
        title.setText(BRAND);
        title.setTextColor(Color.WHITE);
        title.setTypeface(Typeface.DEFAULT_BOLD);
        title.setTextSize(TypedValue.COMPLEX_UNIT_SP, 30);
        box.addView(title);

        TextView text = new TextView(this);
        text.setText("OpenXIII needs the files of the original game XIII (2003), which you must own.\n\n"
                + "Please select the original game's XIII.exe on this device, usually in the \"System\" folder of the game "
                + "(or choose the game folder itself). The original game is only read, never modified.");
        text.setTextColor(0xFFCCCCCC);
        text.setTextSize(TypedValue.COMPLEX_UNIT_SP, 16);
        text.setPadding(0, dp(16), 0, dp(16));
        box.addView(text);

        access = button("Allow access to files", v -> askAccess());
        box.addView(access);
        box.addView(button("Select the original game's XIII.exe", v -> pickExe()));
        box.addView(button("Or choose the game folder", v -> pickFolder()));
        play = button("Play", v -> launch());
        box.addView(play);
        updates = button("", v -> {
            UpdateNotice.setEnabled(prefs, !UpdateNotice.enabled(prefs));
            refresh();
        });
        box.addView(updates);

        status = new TextView(this);
        status.setTextColor(0xFFE8B000);
        status.setTextSize(TypedValue.COMPLEX_UNIT_SP, 15);
        status.setPadding(0, dp(16), 0, 0);
        box.addView(status);

        ScrollView scroll = new ScrollView(this);
        scroll.setBackgroundColor(Color.BLACK);
        scroll.addView(box);
        setContentView(scroll);
        refresh();
    }

    private Button button(String label, android.view.View.OnClickListener l) {
        Button b = new Button(this);
        b.setText(label);
        b.setAllCaps(false);
        b.setOnClickListener(l);
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(dp(320), LinearLayout.LayoutParams.WRAP_CONTENT);
        lp.topMargin = dp(8);
        b.setLayoutParams(lp);
        return b;
    }

    private void refresh() {
        boolean ok = hasAccess();
        access.setVisibility(ok ? android.view.View.GONE : android.view.View.VISIBLE);
        File dir = gameDir();
        play.setEnabled(ok && dir != null);
        updates.setText(UpdateNotice.enabled(prefs) ? "Say when a new version exists: on" : "Say when a new version exists: off");
        if (!ok) {
            status.setText("First, allow OpenXIII to read files on this device.");
        } else if (dir == null) {
            status.setText("No game folder chosen yet.");
        } else {
            status.setText("Game folder: " + dir.getPath());
        }
    }

    private int dp(int v) {
        return (int) TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, getResources().getDisplayMetrics());
    }

    // ----- File access -----

    private boolean hasAccess() {
        if (Build.VERSION.SDK_INT >= 30) return Environment.isExternalStorageManager();
        return checkSelfPermission(Manifest.permission.READ_EXTERNAL_STORAGE) == PackageManager.PERMISSION_GRANTED;
    }

    private void askAccess() {
        if (Build.VERSION.SDK_INT >= 30) {
            try {
                startActivity(new Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                        Uri.parse("package:" + getPackageName())));
            } catch (Exception e) {
                startActivity(new Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION));
            }
        } else {
            requestPermissions(new String[]{Manifest.permission.READ_EXTERNAL_STORAGE}, ASK_STORAGE);
        }
    }

    @Override
    public void onRequestPermissionsResult(int code, String[] perms, int[] results) {
        super.onRequestPermissionsResult(code, perms, results);
        refresh();
    }

    // ----- Game folder -----

    private void pickFolder() {
        Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
        startActivityForResult(i, PICK_FOLDER);
    }

    /** As on PC: the original game's XIII.exe; its folder (or the one above) is the game. */
    private void pickExe() {
        Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        i.addCategory(Intent.CATEGORY_OPENABLE);
        i.setType("*/*");
        startActivityForResult(i, PICK_EXE);
    }

    @Override
    protected void onActivityResult(int code, int result, Intent data) {
        super.onActivityResult(code, result, data);
        if (result != RESULT_OK || data == null || data.getData() == null) return;
        File chosen;
        if (code == PICK_FOLDER) {
            chosen = pathOfTree(data.getData());
        } else if (code == PICK_EXE) {
            File exe = pathOfDocument(data.getData());
            chosen = exe == null ? null : exe.getParentFile();
        } else {
            return;
        }
        File root = chosen == null ? null : findGame(chosen);
        if (root == null) {
            status.setText("This does not look like the original XIII game folder "
                    + "(System/Engine.u, System/XIII.ini or Maps/MapMenu.unr is missing)."
                    + (chosen != null ? "\n\nChosen: " + chosen.getPath()
                    : "\n\nPlease browse to it from the device's storage (not from \"Recent\" or \"Downloads\")."));
            return;
        }
        prefs.edit().putString("gameDir", root.getPath()).apply();
        refresh();
        if (hasAccess()) launch();
    }

    /** File-system folder of a folder chosen in Android's picker. */
    private static File pathOfTree(Uri tree) {
        try {
            return pathOfId(DocumentsContract.getTreeDocumentId(tree));
        } catch (Exception e) {
            return null;
        }
    }

    /** File-system file of a file chosen in Android's picker. */
    private static File pathOfDocument(Uri doc) {
        try {
            return pathOfId(DocumentsContract.getDocumentId(doc));
        } catch (Exception e) {
            return null;
        }
    }

    /** "primary:Games/XIII" → /storage/emulated/0/Games/XIII; "1234-ABCD:…" → SD card. */
    private static File pathOfId(String id) {
        if (id.startsWith("raw:")) return new File(id.substring(4));
        int colon = id.indexOf(':');
        String volume = colon < 0 ? id : id.substring(0, colon);
        String rel = colon < 0 ? "" : id.substring(colon + 1);
        File base;
        if (volume.equals("primary")) base = Environment.getExternalStorageDirectory();
        else if (volume.equals("home")) base = new File(Environment.getExternalStorageDirectory(), "Documents");
        else base = new File("/storage/" + volume);
        return rel.isEmpty() ? base : new File(base, rel);
    }

    private static File child(File dir, String name) {
        File[] list = dir.listFiles();
        if (list == null) return null;
        for (File f : list) if (f.getName().equalsIgnoreCase(name)) return f;
        return null;
    }

    private static boolean isGame(File root) {
        File sys = child(root, "System");
        File maps = child(root, "Maps");
        return sys != null && maps != null && child(sys, "Engine.u") != null
                && child(sys, "XIII.ini") != null && child(maps, "MapMenu.unr") != null;
    }

    /** The game folder: the one chosen, its parent (System chosen) or one of its sub-folders. */
    private static File findGame(File chosen) {
        if (isGame(chosen)) return chosen;
        File parent = chosen.getParentFile();
        if (parent != null && isGame(parent)) return parent;
        File[] list = chosen.listFiles();
        if (list != null) for (File f : list) if (f.isDirectory() && isGame(f)) return f;
        return null;
    }

    private File gameDir() {
        String p = prefs.getString("gameDir", null);
        if (p == null) return null;
        File f = new File(p);
        return hasAccess() && !isGame(f) ? null : f;
    }

    // ----- Launch -----

    /** OpenXIII.ini read by launcher.rs, in <filesDir>/OpenXIII. */
    private boolean writeIni(File dir) {
        File d = new File(getFilesDir(), "OpenXIII");
        d.mkdirs();
        try (FileOutputStream out = new FileOutputStream(new File(d, "OpenXIII.ini"))) {
            String ini = "[OpenXIII]\r\nGameDir=" + dir.getPath() + "\r\n";
            File mods = modsDir();
            if (mods != null) ini += "ModsDir=" + mods.getPath() + "\r\n";
            out.write(ini.getBytes(StandardCharsets.UTF_8));
            return true;
        } catch (Exception e) {
            if (status != null) status.setText("Could not save the settings: " + e.getMessage());
            return false;
        }
    }

    // ----- Mods (improved build) -----

    /** Example mods packed in the APK (improved build only): assets/mod-examples/<mod>/... */
    private static final String EXAMPLES_ASSETS = "mod-examples";

    /**
     * Improved build: the OpenXIII folder of shared storage, as the PC's game folder: "Mods" (the
     * mods a computer copies over USB) and, inside it, "_Examples" (the examples shipped with this
     * version, replaced when the app is updated, with the folders earlier versions shipped; never
     * loaded until one is copied into Mods).
     * Null in the faithful build (no mods).
     */
    private File modsDir() {
        String[] examples;
        try {
            examples = getAssets().list(EXAMPLES_ASSETS);
        } catch (IOException e) {
            return null;
        }
        if (examples == null || examples.length == 0) return null;
        File root = new File(Environment.getExternalStorageDirectory(), "OpenXIII");
        File mods = new File(root, "Mods");
        mods.mkdirs();
        try {
            long stamp = getPackageManager().getPackageInfo(getPackageName(), 0).lastUpdateTime;
            File marker = new File(getFilesDir(), "mod-examples-" + stamp);
            File target = new File(mods, "_Examples");
            if (!marker.exists() || !target.isDirectory()) {
                // Mods/_Examples is the app's own: replaced as a whole at each new version. The
                // folders earlier versions shipped (the examples and the guide next to Mods, the
                // old examples folder inside it) are moved aside next to Mods, never deleted; once
                // moved they are not found again. The player's own mods in Mods are never touched.
                deleteTree(target);
                String moved = "-old-" + new java.text.SimpleDateFormat("yyyyMMdd-HHmm", java.util.Locale.ROOT).format(new java.util.Date());
                File[][] old = {
                    {new File(root, "Mod examples"), new File(root, "Mod examples" + moved)},
                    {new File(root, "Modding"), new File(root, "Modding" + moved)},
                    {new File(mods, "examples"), new File(root, "examples" + moved)},
                };
                for (File[] o : old) moveAside(o[0], o[1]);
                copyAssets(EXAMPLES_ASSETS, target);
                marker.createNewFile();
            }
        } catch (Exception e) {
            if (status != null) status.setText("Could not copy the example mods: " + e.getMessage());
        }
        return mods;
    }

    /**
     * Moves folder `from` to `to` (or `to` with "-2", "-3"... when taken), if it is there. When
     * the move fails, the folder stays where it is and the failure is logged: nothing is deleted.
     */
    private static void moveAside(File from, File to) {
        if (!from.exists()) return;
        File dest = to;
        for (int n = 2; dest.exists() && n < 100; n++) dest = new File(to.getPath() + "-" + n);
        if (!from.renameTo(dest)) {
            android.util.Log.w("OpenXIII", "could not move " + from + " to " + dest + ": left where it is");
        }
    }

    /** Deletes `f` (Mods/_Examples, the app's own) and what it holds; a symbolic link is removed, never followed. */
    private static void deleteTree(File f) throws IOException {
        if (!f.exists() && !java.nio.file.Files.isSymbolicLink(f.toPath())) return;
        if (f.isDirectory() && !java.nio.file.Files.isSymbolicLink(f.toPath())) {
            File[] list = f.listFiles();
            if (list != null) for (File c : list) deleteTree(c);
        }
        if (!f.delete()) throw new IOException("cannot delete " + f);
    }

    /** Copies the assets folder `from` into `to` (files replaced, other files kept). */
    private void copyAssets(String from, File to) throws IOException {
        String[] names = getAssets().list(from);
        if (names == null || names.length == 0) {
            to.getParentFile().mkdirs();
            try (InputStream in = getAssets().open(from); OutputStream out = new FileOutputStream(to)) {
                byte[] buf = new byte[65536];
                for (int n; (n = in.read(buf)) > 0; ) out.write(buf, 0, n);
            }
            return;
        }
        to.mkdirs();
        for (String n : names) copyAssets(from + "/" + n, new File(to, n));
    }

    private void launch() {
        File dir = gameDir();
        if (dir == null || !writeIni(dir)) return;
        startActivity(new Intent(this, GameActivityXiii.class));
        finish();
    }

    /** The game runs in its own process (":game"): stopped before changing folders. */
    private void stopGame() {
        ActivityManager am = (ActivityManager) getSystemService(ACTIVITY_SERVICE);
        if (am == null || am.getRunningAppProcesses() == null) return;
        for (ActivityManager.RunningAppProcessInfo p : am.getRunningAppProcesses()) {
            if (p.processName.equals(getPackageName() + ":game")) android.os.Process.killProcess(p.pid);
        }
    }
}

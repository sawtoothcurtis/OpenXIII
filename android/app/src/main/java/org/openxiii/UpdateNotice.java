package org.openxiii;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.Intent;
import android.content.SharedPreferences;
import android.net.Uri;
import android.os.Handler;
import android.os.Looper;

import java.net.HttpURLConnection;
import java.net.URL;

/**
 * New version notice of the launcher, like crates/openxiii-update on PC: at launch, before the game
 * starts, it asks GitHub which release is the latest and, when it is newer than this app, says so:
 * "Open the download page", "Later" or "Don't ask again". Nothing is downloaded or installed here:
 * the release page opens in the browser. Nothing is sent about the player or the device.
 */
final class UpdateNotice {
    static final String REPOSITORY = "https://github.com/sawtoothcurtis/OpenXIII";
    private static final String PREF_AUTO = "updateAuto";

    private UpdateNotice() {}

    static boolean enabled(SharedPreferences prefs) {
        return prefs.getBoolean(PREF_AUTO, true);
    }

    static void setEnabled(SharedPreferences prefs, boolean on) {
        prefs.edit().putBoolean(PREF_AUTO, on).apply();
    }

    /** Says when a newer version exists, then calls {@code then} on the main thread (unless the page is opened). */
    static void atLaunch(Activity activity, SharedPreferences prefs, Runnable then) {
        if (!enabled(prefs)) {
            then.run();
            return;
        }
        Handler main = new Handler(Looper.getMainLooper());
        new Thread(() -> {
            String found = null;
            try {
                String current = activity.getPackageManager().getPackageInfo(activity.getPackageName(), 0).versionName;
                String latest = latest();
                if (latest != null && isNewer(latest, current)) found = latest;
            } catch (Exception e) {
                android.util.Log.i("OpenXIII", "update: could not look for a new version: " + e);
            }
            String version = found;
            main.post(() -> {
                if (version == null || activity.isFinishing()) then.run();
                else ask(activity, prefs, version, then);
            });
        }, "OpenXIII update notice").start();
    }

    private static void ask(Activity activity, SharedPreferences prefs, String version, Runnable then) {
        new AlertDialog.Builder(activity)
                .setTitle("OpenXIII " + version + " is available")
                .setMessage("The new version can be downloaded from the official OpenXIII page on GitHub.")
                .setPositiveButton("Open the download page", (d, w) -> {
                    try {
                        activity.startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(REPOSITORY + "/releases/tag/v" + version)));
                        activity.finish();
                    } catch (Exception e) {
                        then.run();
                    }
                })
                .setNegativeButton("Later", (d, w) -> then.run())
                .setNeutralButton("Don't ask again", (d, w) -> {
                    setEnabled(prefs, false);
                    then.run();
                })
                .setOnCancelListener(d -> then.run())
                .show();
    }

    /** Version of the latest release: where ".../releases/latest" redirects to (".../releases/tag/v0.2.0"). */
    private static String latest() throws Exception {
        HttpURLConnection c = (HttpURLConnection) new URL(REPOSITORY + "/releases/latest").openConnection();
        try {
            c.setInstanceFollowRedirects(false);
            c.setRequestMethod("HEAD");
            c.setConnectTimeout(4000);
            c.setReadTimeout(8000);
            int code = c.getResponseCode();
            String location = c.getHeaderField("Location");
            if (code < 300 || code >= 400 || location == null) return null;
            String prefix = REPOSITORY + "/releases/tag/";
            if (!location.startsWith(prefix)) return null;
            String tag = location.substring(prefix.length());
            String v = tag.startsWith("v") ? tag.substring(1) : tag;
            return version(v) != null ? v : null;
        } finally {
            c.disconnect();
        }
    }

    /** "0.2.0" > "0.1"; anything that is not one to four numbers is never newer. */
    static boolean isNewer(String candidate, String current) {
        int[] a = version(candidate), b = version(current);
        if (a == null || b == null) return false;
        for (int i = 0; i < 4; i++) if (a[i] != b[i]) return a[i] > b[i];
        return false;
    }

    private static int[] version(String v) {
        if (v == null || !v.matches("[0-9]{1,9}(\\.[0-9]{1,9}){0,3}")) return null;
        String[] p = v.split("\\.");
        int[] out = new int[4];
        for (int i = 0; i < p.length; i++) out[i] = Integer.parseInt(p[i]);
        return out;
    }
}

package com.frstycodes.hum;

import android.content.Context;
import android.content.SharedPreferences;
import android.os.Build;

import java.io.File;
import java.io.FileOutputStream;
import java.io.PrintWriter;
import java.io.StringWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;

/**
 * If Hum crashes, the stack trace is saved; the next launch sends it to Hum on the PC
 * first thing (it lands as hum-android-crash.txt beside hum.exe) and shows it.
 */
final class Crash {
    private static volatile boolean installed;

    static void install(Context context) {
        if (installed) return;
        installed = true;
        final Context app = context.getApplicationContext();
        final Thread.UncaughtExceptionHandler next = Thread.getDefaultUncaughtExceptionHandler();
        Thread.setDefaultUncaughtExceptionHandler((thread, error) -> {
            save(app, format(thread.getName(), error));
            if (next != null) next.uncaughtException(thread, error);
        });
    }

    static String format(String where, Throwable error) {
        StringWriter sw = new StringWriter();
        PrintWriter pw = new PrintWriter(sw);
        pw.println("Hum " + BuildInfo.VERSION + " on Android " + Build.VERSION.RELEASE + " (API " + Build.VERSION.SDK_INT + "), " + Build.MANUFACTURER + " " + Build.MODEL);
        pw.println("where: " + where);
        error.printStackTrace(pw);
        pw.flush();
        return sw.toString();
    }

    static void save(Context c, String report) {
        try (FileOutputStream out = new FileOutputStream(file(c))) {
            out.write(report.getBytes(StandardCharsets.UTF_8));
        } catch (Throwable ignored) {
        }
    }

    /** The saved report, still on disk; null if there isn't one. */
    static String peek(Context c) {
        File f = file(c);
        if (!f.exists()) return null;
        try {
            return new String(Files.readAllBytes(f.toPath()), StandardCharsets.UTF_8);
        } catch (Exception e) {
            return null;
        }
    }

    /** The saved report, removed; null if there isn't one. */
    static String take(Context c) {
        String s = peek(c);
        file(c).delete();
        return s;
    }

    /** Sends a report to the paired PC on its own thread (no-op when not paired). */
    static void send(Context c, String report) {
        SharedPreferences p = Prefs.of(c);
        String host = p.getString(Prefs.HOST, "");
        int port = p.getInt(Prefs.PORT, 8443);
        String fp = p.getString(Prefs.FINGERPRINT, "");
        if (host.isEmpty() || report == null) return;
        new Thread(() -> {
            try (Ws ws = Ws.connect(host, port, fp)) {
                ws.sendText("crash\n" + report);
            } catch (Throwable ignored) {
            }
        }, "hum-report").start();
    }

    private static File file(Context c) {
        return new File(c.getFilesDir(), "crash.txt");
    }

    private Crash() {
    }
}

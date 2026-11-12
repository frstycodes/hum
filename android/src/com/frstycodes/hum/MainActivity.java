package com.frstycodes.hum;

import android.Manifest;
import android.app.Activity;
import android.app.AlertDialog;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.content.res.Configuration;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Insets;
import android.graphics.LinearGradient;
import android.graphics.Paint;
import android.graphics.RectF;
import android.graphics.Shader;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.text.InputType;
import android.util.TypedValue;
import android.view.Choreographer;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.TextView;

/**
 * Hum's one screen, in Hum's look (Stash's palette, DM Sans, nothing boxed): the logo
 * riding your voice, a status line, the level meter, which PC you're paired with, the
 * noise suppression switch, and Start/Stop. Pairing comes from the phone page's
 * "Use the Hum app" (hum://pc:port/?k=fingerprint) or from typing the PC's address.
 */
public final class MainActivity extends Activity {
    private static final int ASK_MIC = 1;

    // palette (set per theme in onCreate)
    private int bg, text, dim, faint, track, soft, lo, hi;
    private static final int ORANGE = 0xFFF27D2E, GREEN = 0xFF30D158, ROSE = 0xFFD1384B;
    private Typeface font;

    private LogoView logo;
    private View dot;
    private TextView status, pcValue, pcAction, hint, go;
    private MeterView meter;
    private SwitchView noise;

    private float level;
    private final float[] past = new float[13];
    private boolean resumed;
    private boolean startWhenAllowed;

    // ---------- lifecycle ----------

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        Crash.install(this);
        boolean dark = (getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES;
        bg = dark ? 0xFF1F1B23 : 0xFFF4F2F6;
        text = dark ? 0xFFF4F1F6 : 0xFF1C1920;
        dim = dark ? 0xFFA59FAD : 0xFF6B6572;
        faint = dark ? 0x52FFFFFF : 0x6B1C1920;
        track = dark ? 0x1AFFFFFF : 0x1A1C1920;
        soft = dark ? 0x1CFFFFFF : 0x141C1920;
        lo = dark ? Color.rgb(255, 196, 150) : Color.rgb(247, 178, 122);
        hi = dark ? Color.rgb(226, 98, 22) : Color.rgb(184, 70, 8);
        try {
            font = Typeface.createFromAsset(getAssets(), "DMSans.ttf");
            // the window's decor (and its insets controller) only exists once there's content
            setContentView(build());
            edgeToEdge(dark);
            handle(getIntent());
        } catch (Throwable t) {
            // don't die on start: show what went wrong, and get it to the PC
            String report = Crash.format("starting up", t);
            Crash.save(this, report);
            showError(report);
        }
        // a crash from last time goes to the PC first thing (now that pairing is known)
        Crash.send(this, Crash.peek(this));
    }

    /** The bare-bones screen for when the real one couldn't be built. */
    private void showError(String report) {
        broken = true;
        TextView t = new TextView(this);
        t.setText("Hum couldn't start. A copy of this went to Hum on your PC.\n\n" + report);
        t.setTextColor(text);
        t.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
        t.setTextIsSelectable(true);
        t.setPadding(dp(20), dp(48), dp(20), dp(48));
        android.widget.ScrollView s = new android.widget.ScrollView(this);
        s.setBackgroundColor(bg);
        s.addView(t);
        setContentView(s);
    }

    private boolean broken;

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        handle(intent);
    }

    @Override
    protected void onResume() {
        super.onResume();
        if (broken) return;
        resumed = true;
        Choreographer.getInstance().postFrameCallback(this::frame);
        refresh();
        reportCrash();
        // Android only lets the mic service start once Hum is really on screen
        if (startOnResume) {
            startOnResume = false;
            if (!MicService.running) start();
        }
    }

    private boolean startOnResume;

    /** Last time Hum crashed: show what happened, and send it to the PC for a fix. */
    private void reportCrash() {
        String report = Crash.take(this); // already sent to the PC in onCreate
        if (report == null) return;
        String host = prefs().getString(Prefs.HOST, "");
        TextView body = label(report, 12, 400, dim);
        body.setTextIsSelectable(true);
        body.setPadding(dp(20), dp(8), dp(20), 0);
        android.widget.ScrollView scroll = new android.widget.ScrollView(this);
        scroll.addView(body);
        new AlertDialog.Builder(this)
                .setTitle("Hum stopped last time")
                .setMessage(host.isEmpty() ? "Here's what went wrong." : "Here's what went wrong. A copy went to Hum on your PC.")
                .setView(scroll)
                .setPositiveButton("OK", null)
                .setNeutralButton("Copy", (d, w) -> {
                    android.content.ClipboardManager cm = getSystemService(android.content.ClipboardManager.class);
                    if (cm != null) cm.setPrimaryClip(android.content.ClipData.newPlainText("Hum crash", report));
                })
                .show();
    }

    @Override
    protected void onPause() {
        super.onPause();
        resumed = false;
    }

    // ---------- pairing ----------

    /** hum://192.168.0.103:8443/?k=<fingerprint>, from the phone page's button. */
    private void handle(Intent intent) {
        Uri u = intent == null ? null : intent.getData();
        if (u == null || !"hum".equals(u.getScheme()) || u.getHost() == null) return;
        String fp = u.getQueryParameter("k");
        boolean samePc = u.getHost().equals(prefs().getString(Prefs.HOST, "")) && fp != null && fp.equalsIgnoreCase(prefs().getString(Prefs.FINGERPRINT, ""));
        prefs().edit()
                .putString(Prefs.HOST, u.getHost())
                .putInt(Prefs.PORT, u.getPort() > 0 ? u.getPort() : 8443)
                .putString(Prefs.FINGERPRINT, fp == null ? "" : fp.toLowerCase())
                .apply();
        setIntent(new Intent(this, MainActivity.class)); // don't re-pair on rotation
        if (MicService.running && !samePc) stopService();
        startOnResume = true;
        refresh();
    }

    private SharedPreferences prefs() {
        return Prefs.of(this);
    }

    private boolean paired() {
        return !prefs().getString(Prefs.HOST, "").isEmpty();
    }

    private void askAddress() {
        EditText field = new EditText(this);
        field.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_URI);
        field.setHint("192.168.0.103:8443");
        String host = prefs().getString(Prefs.HOST, "");
        if (!host.isEmpty()) field.setText(host + ":" + prefs().getInt(Prefs.PORT, 8443));
        FrameLayout box = new FrameLayout(this);
        box.setPadding(dp(20), dp(8), dp(20), 0);
        box.addView(field);
        new AlertDialog.Builder(this)
                .setTitle("Your PC's address")
                .setMessage("It's under the QR code in Hum on your PC.")
                .setView(box)
                .setNegativeButton("Cancel", null)
                .setPositiveButton("Save", (d, w) -> {
                    String s = field.getText().toString().trim().replaceFirst("^https?://", "").replaceAll("/.*$", "");
                    if (s.isEmpty()) return;
                    String h = s;
                    int port = 8443;
                    int colon = s.lastIndexOf(':');
                    if (colon > 0) {
                        h = s.substring(0, colon);
                        try {
                            port = Integer.parseInt(s.substring(colon + 1));
                        } catch (NumberFormatException ignored) {
                        }
                    }
                    // a new address: trust whatever PC answers there first
                    prefs().edit().putString(Prefs.HOST, h).putInt(Prefs.PORT, port).putString(Prefs.FINGERPRINT, "").apply();
                    if (MicService.running) stopService();
                    refresh();
                })
                .show();
    }

    // ---------- start / stop ----------

    private void start() {
        if (!paired()) {
            askAddress();
            return;
        }
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED
                || (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED && !prefs().getBoolean("askedNotif", false))) {
            startWhenAllowed = true;
            prefs().edit().putBoolean("askedNotif", true).apply();
            String[] want = Build.VERSION.SDK_INT >= 33
                    ? new String[] {Manifest.permission.RECORD_AUDIO, Manifest.permission.POST_NOTIFICATIONS}
                    : new String[] {Manifest.permission.RECORD_AUDIO};
            requestPermissions(want, ASK_MIC);
            return;
        }
        MicService.state = MicService.CONNECTING;
        MicService.detail = "Connecting to your PC…";
        try {
            startForegroundService(new Intent(this, MicService.class));
        } catch (RuntimeException e) {
            MicService.state = MicService.FAILED;
            MicService.detail = "Couldn't start: " + e.getClass().getSimpleName();
        }
        refresh();
    }

    private void stopService() {
        startOnResume = false;
        stopService(new Intent(this, MicService.class));
        MicService.running = false;
        MicService.state = MicService.IDLE;
        refresh();
    }

    @Override
    public void onRequestPermissionsResult(int code, String[] perms, int[] results) {
        if (code != ASK_MIC) return;
        boolean mic = checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED;
        if (mic && startWhenAllowed) start();
        else if (!mic) {
            MicService.state = MicService.FAILED;
            MicService.detail = "Hum needs the microphone. Allow it in Settings, Apps, Hum.";
        }
        startWhenAllowed = false;
        refresh();
    }

    // ---------- the screen ----------

    private View build() {
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setBackgroundColor(bg);
        int side = dp(24);
        root.setOnApplyWindowInsetsListener((v, insets) -> {
            int top, bottom;
            if (Build.VERSION.SDK_INT >= 30) {
                Insets i = insets.getInsets(WindowInsets.Type.systemBars() | WindowInsets.Type.ime());
                top = i.top;
                bottom = i.bottom;
            } else {
                top = insets.getSystemWindowInsetTop();
                bottom = insets.getSystemWindowInsetBottom();
            }
            v.setPadding(side, top + dp(20), side, bottom + dp(20));
            return Build.VERSION.SDK_INT >= 30 ? WindowInsets.CONSUMED : insets.consumeSystemWindowInsets();
        });

        // header: logo, name, status
        LinearLayout head = new LinearLayout(this);
        head.setGravity(Gravity.CENTER_VERTICAL);
        logo = new LogoView(this);
        head.addView(logo, new LinearLayout.LayoutParams(dp(36), dp(36)));
        LinearLayout titles = new LinearLayout(this);
        titles.setOrientation(LinearLayout.VERTICAL);
        titles.addView(label("Hum", 20, 600, text));
        LinearLayout statusRow = new LinearLayout(this);
        statusRow.setGravity(Gravity.CENTER_VERTICAL);
        dot = new View(this);
        LinearLayout.LayoutParams dotLp = new LinearLayout.LayoutParams(dp(7), dp(7));
        dotLp.rightMargin = dp(7);
        statusRow.addView(dot, dotLp);
        status = label("", 14, 400, dim);
        statusRow.addView(status);
        titles.addView(statusRow);
        LinearLayout.LayoutParams tLp = new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1);
        tLp.leftMargin = dp(12);
        head.addView(titles, tLp);
        root.addView(head);

        // level
        root.addView(label("Level", 13, 500, faint), margins(0, dp(40), 0, dp(10)));
        meter = new MeterView(this);
        root.addView(meter, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(8)));

        // paired PC
        root.addView(label("PC", 13, 500, faint), margins(0, dp(32), 0, dp(4)));
        LinearLayout pcRow = new LinearLayout(this);
        pcRow.setGravity(Gravity.CENTER_VERTICAL);
        pcValue = label("", 16, 500, text);
        pcRow.addView(pcValue, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1));
        pcAction = label("Change", 15, 500, ORANGE);
        pcAction.setPadding(dp(12), dp(10), 0, dp(10));
        pcAction.setOnClickListener(v -> askAddress());
        pcRow.addView(pcAction);
        root.addView(pcRow);

        // noise suppression
        LinearLayout nsRow = new LinearLayout(this);
        nsRow.setGravity(Gravity.CENTER_VERTICAL);
        LinearLayout nsText = new LinearLayout(this);
        nsText.setOrientation(LinearLayout.VERTICAL);
        nsText.addView(label("Noise suppression", 16, 500, text));
        nsText.addView(label("Takes effect next time you start.", 13, 400, dim));
        nsRow.addView(nsText, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1));
        noise = new SwitchView(this);
        noise.on = prefs().getBoolean(Prefs.NOISE, true);
        nsRow.addView(noise, new LinearLayout.LayoutParams(dp(44), dp(26)));
        nsRow.setOnClickListener(v -> {
            noise.on = !noise.on;
            prefs().edit().putBoolean(Prefs.NOISE, noise.on).apply();
            noise.invalidate();
        });
        root.addView(nsRow, margins(0, dp(24), 0, 0));

        root.addView(new View(this), new LinearLayout.LayoutParams(1, 0, 1));

        // start / stop
        go = label("Start", 16, 600, Color.WHITE);
        go.setGravity(Gravity.CENTER);
        go.setOnClickListener(v -> {
            if (MicService.running) stopService();
            else start();
        });
        pressable(go);
        root.addView(go, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(54)));
        hint = label("", 13, 400, dim);
        hint.setGravity(Gravity.CENTER);
        root.addView(hint, margins(dp(8), dp(12), dp(8), 0));
        return root;
    }

    /** Status, PC, button and hint from the service's state. */
    private void refresh() {
        if (status == null) return;
        int s = MicService.running ? MicService.state : (MicService.state == MicService.FAILED ? MicService.FAILED : MicService.IDLE);
        String detail = MicService.detail;
        int dotColor;
        switch (s) {
            case MicService.STREAMING: status.setText("Streaming to your PC"); dotColor = GREEN; break;
            case MicService.CONNECTING: status.setText("Connecting…"); dotColor = dim; break;
            case MicService.RETRYING: status.setText(detail); dotColor = ORANGE; break;
            case MicService.FAILED: status.setText(detail); dotColor = ROSE; break;
            default: status.setText(paired() ? "Not streaming" : "Not paired yet"); dotColor = dim;
        }
        GradientDrawable d = new GradientDrawable();
        d.setShape(GradientDrawable.OVAL);
        d.setColor(dotColor);
        dot.setBackground(d);

        String host = prefs().getString(Prefs.HOST, "");
        pcValue.setText(host.isEmpty() ? "None yet" : host);
        pcValue.setTextColor(host.isEmpty() ? dim : text);
        pcAction.setText(host.isEmpty() ? "Enter address" : "Change");

        boolean live = MicService.running;
        go.setText(live ? "Stop" : "Start");
        go.setTextColor(live ? text : Color.WHITE);
        GradientDrawable fill = new GradientDrawable();
        fill.setCornerRadius(dp(27));
        fill.setColor(live ? soft : ORANGE);
        go.setBackground(fill);

        hint.setText(!paired()
                ? "On your PC, scan Hum's QR with your camera and tap “Use the Hum app” on the page. Or enter the address."
                : live ? "Keeps going with the screen locked. Stop it here or from the notification."
                : "Your phone keeps working as a mic with the screen locked.");
    }

    /** 60 fps while on screen: the meter and the logo ride the level. */
    private void frame(long nanos) {
        if (!resumed) return;
        float p = MicService.peak;
        MicService.peak = 0;
        level = Math.max(level * 0.85f, (float) Math.sqrt(p));
        if (!MicService.running) level *= 0.8f;
        System.arraycopy(past, 0, past, 1, past.length - 1);
        past[0] = level;
        meter.level = level;
        meter.invalidate();
        logo.invalidate();
        if (frameCount++ % 15 == 0) refresh();
        Choreographer.getInstance().postFrameCallback(this::frame);
    }

    private int frameCount;

    // ---------- helpers ----------

    private TextView label(String s, int sp, int weight, int color) {
        TextView t = new TextView(this);
        t.setText(s);
        t.setTextSize(TypedValue.COMPLEX_UNIT_SP, sp);
        t.setTypeface(font);
        t.setFontVariationSettings("'wght' " + weight);
        t.setTextColor(color);
        t.setIncludeFontPadding(false);
        t.setLineSpacing(0, 1.25f);
        return t;
    }

    private LinearLayout.LayoutParams margins(int l, int t, int r, int b) {
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        lp.setMargins(l, t, r, b);
        return lp;
    }

    private void pressable(View v) {
        v.setOnTouchListener((view, e) -> {
            if (e.getAction() == MotionEvent.ACTION_DOWN) view.animate().scaleX(0.97f).scaleY(0.97f).setDuration(100).start();
            else if (e.getAction() == MotionEvent.ACTION_UP || e.getAction() == MotionEvent.ACTION_CANCEL) view.animate().scaleX(1f).scaleY(1f).setDuration(160).start();
            return false;
        });
    }

    private int dp(float v) {
        return Math.round(v * getResources().getDisplayMetrics().density);
    }

    @SuppressWarnings("deprecation")
    private void edgeToEdge(boolean dark) {
        getWindow().setStatusBarColor(Color.TRANSPARENT);
        getWindow().setNavigationBarColor(Color.TRANSPARENT);
        if (Build.VERSION.SDK_INT >= 30) {
            getWindow().setDecorFitsSystemWindows(false);
            WindowInsetsController c = getWindow().getDecorView().getWindowInsetsController();
            if (c != null) {
                int light = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS | WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;
                c.setSystemBarsAppearance(dark ? 0 : light, light);
            }
        } else {
            int flags = View.SYSTEM_UI_FLAG_LAYOUT_STABLE | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION;
            if (!dark) flags |= View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
            getWindow().getDecorView().setSystemUiVisibility(flags);
        }
    }

    // ---------- custom views ----------

    /** Hum's logo: four rounded bars in Stash's colours, lifting with your voice. */
    private final class LogoView extends View {
        private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final int[] colors = {0xFFF9C74F, 0xFFF27D2E, 0xFFD1384B, 0xFF6A2C70};
        private final float[] cx = {176, 400, 624, 848};
        private final float[] rest = {500, 880, 680, 400};
        private final RectF r = new RectF();

        LogoView(Context c) {
            super(c);
        }

        @Override
        protected void onDraw(Canvas canvas) {
            float s = getWidth() / 1024f;
            for (int i = 0; i < 4; i++) {
                float lift = Math.min(1f, past[Math.min(past.length - 1, i * 4)] * 0.9f);
                float h = rest[i] + (940 - rest[i]) * lift;
                r.set((cx[i] - 77) * s, (512 - h / 2) * s, (cx[i] + 77) * s, (512 + h / 2) * s);
                paint.setColor(colors[i]);
                canvas.drawRoundRect(r, 43.1f * s, 43.1f * s, paint);
            }
        }
    }

    /**
     * Stash's progress bar as a level meter: a filled pill, a 4dp gap, the rest of the
     * track. The fill runs light to deep across the whole width (same scale as Hum's).
     */
    private final class MeterView extends View {
        float level;
        private final Paint fill = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final Paint rest = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final RectF r = new RectF();

        MeterView(Context c) {
            super(c);
            rest.setColor(track);
        }

        @Override
        protected void onSizeChanged(int w, int h, int ow, int oh) {
            fill.setShader(new LinearGradient(0, 0, w, 0, lo, hi, Shader.TileMode.CLAMP));
        }

        @Override
        protected void onDraw(Canvas canvas) {
            float w = getWidth(), h = getHeight(), rad = h / 2f;
            float fw = Math.min(1f, level) * w;
            if (fw < h) {
                r.set(0, 0, w, h);
                canvas.drawRoundRect(r, rad, rad, rest);
                return;
            }
            r.set(0, 0, fw, h);
            canvas.drawRoundRect(r, rad, rad, fill);
            float gap = dp(4);
            if (w - fw - gap >= h) {
                r.set(fw + gap, 0, w, h);
                canvas.drawRoundRect(r, rad, rad, rest);
            }
        }
    }

    /** Stash's switch: a capsule track (orange when on) and a wider-than-tall white knob. */
    private final class SwitchView extends View {
        boolean on;
        private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final RectF r = new RectF();

        SwitchView(Context c) {
            super(c);
        }

        @Override
        protected void onDraw(Canvas canvas) {
            float w = getWidth(), h = getHeight();
            paint.setColor(on ? ORANGE : track | 0x10000000);
            r.set(0, 0, w, h);
            canvas.drawRoundRect(r, h / 2, h / 2, paint);
            float kw = w * 0.6f, kh = h - dp(4), kx = on ? w - kw - dp(2) : dp(2);
            paint.setColor(0x2E000000);
            r.set(kx, dp(2) + dp(1), kx + kw, dp(2) + kh + dp(1));
            canvas.drawRoundRect(r, kh / 2, kh / 2, paint);
            paint.setColor(Color.WHITE);
            r.set(kx, dp(2), kx + kw, dp(2) + kh);
            canvas.drawRoundRect(r, kh / 2, kh / 2, paint);
        }
    }
}

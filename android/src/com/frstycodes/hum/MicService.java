package com.frstycodes.hum;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.ServiceInfo;
import android.media.AudioFormat;
import android.media.AudioRecord;
import android.media.MediaRecorder;
import android.media.audiofx.AutomaticGainControl;
import android.media.audiofx.NoiseSuppressor;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.PowerManager;

import java.io.IOException;

/**
 * Records the mic and streams it to Hum on the PC, as a microphone foreground service:
 * Android lets it keep recording with the screen locked, the way call and recorder
 * apps do, and shows a notification for as long as it runs. Holds a partial wake lock
 * and a Wi-Fi lock so neither the CPU nor the radio naps mid-sentence. Reconnects by
 * itself (backing off up to 5 s) when the PC goes away.
 */
public final class MicService extends Service {
    static final String STOP = "com.frstycodes.hum.STOP";
    private static final String CHANNEL = "listening";
    private static final int RATE = 48000;

    // What the screen shows; the activity reads these (same process).
    static final int IDLE = 0, CONNECTING = 1, STREAMING = 2, RETRYING = 3, FAILED = 4;
    static volatile int state = IDLE;
    static volatile String detail = "";
    static volatile boolean running;
    /** Loudest sample since the screen last looked, 0..1. */
    static volatile float peak;

    private volatile boolean stop;
    private volatile Ws current;
    private Thread worker;
    private PowerManager.WakeLock wake;
    private WifiManager.WifiLock wifi;
    private final Handler main = new Handler(Looper.getMainLooper());

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && STOP.equals(intent.getAction())) {
            stopSelf();
            return START_NOT_STICKY;
        }
        channel();
        Notification n = notification("Connecting to your PC…");
        try {
            if (Build.VERSION.SDK_INT >= 29) {
                startForeground(1, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE);
            } else {
                startForeground(1, n);
            }
        } catch (RuntimeException e) {
            // Android refused the microphone service (usually: not started from the open app)
            state = FAILED;
            detail = "Android didn't let Hum use the mic in the background. Open Hum and tap Start.";
            stopSelf();
            return START_NOT_STICKY;
        }
        if (worker == null) {
            stop = false;
            running = true;
            worker = new Thread(this::work, "hum-mic");
            worker.start();
        }
        return START_NOT_STICKY;
    }

    @Override
    public void onCreate() {
        super.onCreate();
        Crash.install(this);
    }

    @Override
    public void onDestroy() {
        stop = true;
        Ws ws = current;
        // closing writes to the network, which Android forbids on the main thread
        if (ws != null) new Thread(ws::close, "hum-close").start();
        if (worker != null) worker.interrupt();
        running = false;
        if (state != FAILED) state = IDLE;
        super.onDestroy();
    }

    private void work() {
        SharedPreferences p = Prefs.of(this);
        String host = p.getString(Prefs.HOST, "");
        int port = p.getInt(Prefs.PORT, 8443);
        String pinned = p.getString(Prefs.FINGERPRINT, "");

        PowerManager pm = (PowerManager) getSystemService(Context.POWER_SERVICE);
        wake = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "hum:mic");
        wake.acquire();
        WifiManager wm = (WifiManager) getApplicationContext().getSystemService(Context.WIFI_SERVICE);
        wifi = wm.createWifiLock(Build.VERSION.SDK_INT >= 29 ? WifiManager.WIFI_MODE_FULL_LOW_LATENCY : WifiManager.WIFI_MODE_FULL_HIGH_PERF, "hum:wifi");
        wifi.acquire();

        AudioRecord rec = null;
        try {
            int min = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT);
            rec = new AudioRecord(MediaRecorder.AudioSource.MIC, RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, Math.max(min, RATE / 5 * 2));
            if (rec.getState() != AudioRecord.STATE_INITIALIZED) throw new IOException("Couldn't open the microphone");
            int session = rec.getAudioSessionId();
            if (p.getBoolean(Prefs.NOISE, true) && NoiseSuppressor.isAvailable()) {
                NoiseSuppressor ns = NoiseSuppressor.create(session);
                if (ns != null) ns.setEnabled(true);
            }
            if (AutomaticGainControl.isAvailable()) {
                AutomaticGainControl agc = AutomaticGainControl.create(session);
                if (agc != null) agc.setEnabled(true);
            }
            rec.startRecording();

            short[] pcm = new short[480]; // 10 ms
            byte[] bytes = new byte[960];
            int backoff = 500;
            boolean first = true;
            while (!stop) {
                set(first ? CONNECTING : RETRYING, first ? "Connecting to your PC…" : "Reconnecting…");
                first = false;
                try (Ws ws = Ws.connect(host, port, pinned)) {
                    current = ws;
                    if (pinned.isEmpty() && ws.fingerprint != null) {
                        // paired by address: remember this PC from now on
                        pinned = ws.fingerprint;
                        p.edit().putString(Prefs.FINGERPRINT, pinned).apply();
                    }
                    ws.sendText(Integer.toString(RATE));
                    set(STREAMING, "Streaming to your PC");
                    backoff = 500;
                    while (!stop) {
                        int n = rec.read(pcm, 0, pcm.length);
                        if (n <= 0) continue;
                        int pk = 0;
                        for (int i = 0; i < n; i++) {
                            short s = pcm[i];
                            bytes[2 * i] = (byte) s;
                            bytes[2 * i + 1] = (byte) (s >> 8);
                            pk = Math.max(pk, Math.abs((int) s));
                        }
                        float v = pk / 32768f;
                        if (v > peak) peak = v;
                        ws.sendBinary(bytes, n * 2);
                    }
                } catch (Ws.WrongPc e) {
                    set(FAILED, "This isn't the PC you paired with. Pair again from Hum's QR.");
                    break;
                } catch (IOException e) {
                    if (stop) break;
                    set(RETRYING, "Can't reach your PC. Retrying…");
                    // keep the mic drained while waiting, so a reconnect starts fresh
                    long until = System.currentTimeMillis() + backoff;
                    while (!stop && System.currentTimeMillis() < until) rec.read(pcm, 0, pcm.length);
                    backoff = Math.min(backoff * 2, 5000);
                } finally {
                    current = null;
                }
            }
        } catch (IOException | RuntimeException e) {
            set(FAILED, e.getMessage() != null ? e.getMessage() : "Couldn't start");
        } finally {
            if (rec != null) {
                try {
                    rec.stop();
                } catch (RuntimeException ignored) {
                }
                rec.release();
            }
            if (wake.isHeld()) wake.release();
            if (wifi.isHeld()) wifi.release();
            if (!stop) main.post(this::stopSelf); // failed on its own: end the service
        }
    }

    private void set(int s, String text) {
        state = s;
        detail = text;
        NotificationManager nm = getSystemService(NotificationManager.class);
        if (nm != null && s != FAILED) nm.notify(1, notification(text));
    }

    private void channel() {
        NotificationManager nm = getSystemService(NotificationManager.class);
        if (nm != null && nm.getNotificationChannel(CHANNEL) == null) {
            NotificationChannel c = new NotificationChannel(CHANNEL, "While listening", NotificationManager.IMPORTANCE_LOW);
            c.setDescription("Shown while Hum sends your microphone to your PC");
            c.setShowBadge(false);
            nm.createNotificationChannel(c);
        }
    }

    private Notification notification(String text) {
        PendingIntent open = PendingIntent.getActivity(this, 0, new Intent(this, MainActivity.class), PendingIntent.FLAG_IMMUTABLE);
        PendingIntent stopIt = PendingIntent.getService(this, 1, new Intent(this, MicService.class).setAction(STOP), PendingIntent.FLAG_IMMUTABLE);
        Notification.Builder b = new Notification.Builder(this, CHANNEL)
                .setSmallIcon(R.drawable.ic_notif)
                .setContentTitle("Hum is your PC's mic")
                .setContentText(text)
                .setContentIntent(open)
                .setOngoing(true)
                .setColor(0xFFF27D2E)
                .addAction(new Notification.Action.Builder(null, "Stop", stopIt).build());
        if (Build.VERSION.SDK_INT >= 31) b.setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE);
        return b.build();
    }
}

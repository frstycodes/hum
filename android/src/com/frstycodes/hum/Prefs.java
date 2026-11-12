package com.frstycodes.hum;

import android.content.Context;
import android.content.SharedPreferences;

/** The paired PC and the one setting. */
final class Prefs {
    static final String HOST = "host";
    static final String PORT = "port";
    /** SHA-256 of the PC's certificate (hex): the PC this phone trusts. */
    static final String FINGERPRINT = "fingerprint";
    static final String NOISE = "noise";

    static SharedPreferences of(Context c) {
        return c.getSharedPreferences("hum", Context.MODE_PRIVATE);
    }

    private Prefs() {
    }
}

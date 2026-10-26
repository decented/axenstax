package com.axenstax.game;

import android.app.NativeActivity;
import android.content.Intent;

/**
 * NativeActivity that keeps the newest intent. Stock NativeActivity drops
 * onNewIntent, so a link opened while the game runs (singleTask) would leave
 * getIntent() on the launch intent. The native side reads getIntent() over JNI
 * (game/engine/src/android_intent.rs) and consumes the link with setData(null).
 */
public class AxenstaxActivity extends NativeActivity {
    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
    }
}

package com.axenstax.game;

import android.app.NativeActivity;
import android.content.Intent;
import android.os.Bundle;

/**
 * NativeActivity that keeps the newest intent. Stock NativeActivity drops
 * onNewIntent, so a link opened while the game runs (singleTask) would leave
 * getIntent() on the launch intent. The native side reads getIntent() over JNI
 * (game/engine/src/android_intent.rs) and consumes the link with setData(null).
 */
public class AxenstaxActivity extends NativeActivity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        // setData(null) only clears the in-process Intent. When Android recreates the activity
        // (the process was killed in the background, or the task is reopened from Recents) it
        // hands back the task's ORIGINAL launch intent, link included, which would fire a second
        // time. A restored activity is never a fresh tap, so drop the data before the game reads it.
        Intent intent = getIntent();
        if (intent != null
                && (savedInstanceState != null
                    || (intent.getFlags() & Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY) != 0)) {
            intent.setData(null);
        }
        super.onCreate(savedInstanceState);
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
    }
}

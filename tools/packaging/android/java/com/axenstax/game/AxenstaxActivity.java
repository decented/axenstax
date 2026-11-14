package com.axenstax.game;

import android.app.AlertDialog;
import android.app.NativeActivity;
import android.content.DialogInterface;
import android.content.Intent;
import android.os.Bundle;
import android.os.Process;
import android.os.SystemClock;
import android.text.InputFilter;
import android.text.InputType;
import android.view.KeyEvent;
import android.view.Window;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.TextView;

/**
 * The game's NativeActivity. Three jobs, all called from the native side over JNI
 * as instance methods (game/engine/src/android_intent.rs and android_jni.rs):
 *
 * 1. It keeps the newest intent. Stock NativeActivity drops onNewIntent, so a link
 *    opened while the game runs (singleTask) would leave getIntent() on the launch
 *    intent. The native side reads getIntent() and consumes the link with setData(null).
 * 2. The text prompt. NativeActivity has no text-editor view, so Android ignores a
 *    soft-keyboard request; showTextPrompt opens a dark AlertDialog with a one-line
 *    EditText instead. The native thread never waits for it: it polls
 *    textPromptState() while the prompt is open (game/engine/src/text_prompt.rs).
 * 3. The end of the process. The game's event loop can be built once per process, so
 *    a second android_main in a cached process could never start. onDestroy ends the
 *    process, after the game has saved (game/engine/src/android_lifecycle.rs).
 */
public class AxenstaxActivity extends NativeActivity {
    // Text prompt kinds (text_prompt.rs PromptKind::code): 0 name, 1 chat, 2 plain.
    private static final int KIND_NAME = 0;

    // textPromptState() values (android_jni.rs PROMPT_*).
    private static final int PROMPT_NONE = 0;
    private static final int PROMPT_OPEN = 1;
    private static final int PROMPT_DONE = 2;

    /**
     * The longest onDestroy waits for the game to save and end before ending the process
     * anyway: the save, plus the exit function's bounded grace for a handover still being
     * published (world_exit.rs EXIT_GRACE, 10 s; cut short, it resumes at the next start).
     * The game never waits on this thread, so the wait cannot deadlock; at worst it ends.
     */
    private static final long GAME_END_WAIT_MS = 12000;

    private final Object promptLock = new Object();
    private int promptState = PROMPT_NONE; // guarded by promptLock
    private int promptSerial; // guarded by promptLock
    private String promptAnswer; // guarded by promptLock; null = cancelled
    private AlertDialog promptDialog; // UI thread only

    private volatile boolean destroying;
    private final Object endLock = new Object();
    private boolean gameEnded; // guarded by endLock

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

    // ── The text prompt ─────────────────────────────────────────────────────────

    /**
     * Open the text prompt (any thread; the dialog is built on the UI thread). The prompt
     * is OPEN from this call until its dialog is dismissed, which is the one place that
     * posts the answer: OK and Done/Enter post the text, Cancel, Back and a tap outside
     * post "cancelled". A dialog that cannot be built posts "cancelled" too, so the game's
     * keyboard is never left waiting on nothing.
     */
    public void showTextPrompt(final String label, final String initial, final int kind, final int maxLen) {
        final int serial;
        synchronized (promptLock) {
            serial = ++promptSerial;
            promptState = PROMPT_OPEN;
            promptAnswer = null;
        }
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                openPrompt(serial, label, initial, kind, maxLen);
            }
        });
    }

    /** PROMPT_NONE (no prompt known: a cancel to the game), PROMPT_OPEN or PROMPT_DONE. */
    public int textPromptState() {
        synchronized (promptLock) {
            return promptState;
        }
    }

    /** The answer of a PROMPT_DONE prompt (null = cancelled), once; the state goes back to NONE. */
    public String takeTextPromptAnswer() {
        synchronized (promptLock) {
            if (promptState != PROMPT_DONE) {
                return null;
            }
            String answer = promptAnswer;
            promptState = PROMPT_NONE;
            promptAnswer = null;
            return answer;
        }
    }

    /** Post an answer for prompt `serial`; an older dialog's answer is ignored. */
    private void finishPrompt(int serial, String answer) {
        synchronized (promptLock) {
            if (serial != promptSerial || promptState != PROMPT_OPEN) {
                return;
            }
            promptAnswer = answer;
            promptState = PROMPT_DONE;
        }
    }

    private static int inputTypeFor(int kind) {
        if (kind == KIND_NAME) {
            return InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_CAP_WORDS;
        }
        // Chat and plain text: no suggestions or autocorrect, no capitals, so a
        // "/command", a seed or a room code arrives exactly as typed.
        return InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
    }

    private void openPrompt(final int serial, String label, String initial, int kind, int maxLen) {
        if (promptDialog != null) {
            // Not expected (the game asks for one prompt at a time); the old dialog's
            // answer goes to its own serial and is ignored.
            promptDialog.dismiss();
            promptDialog = null;
        }
        try {
            final EditText field = new EditText(this);
            field.setSingleLine(true);
            field.setInputType(inputTypeFor(kind));
            field.setImeOptions(EditorInfo.IME_ACTION_DONE
                    | EditorInfo.IME_FLAG_NO_EXTRACT_UI
                    | EditorInfo.IME_FLAG_NO_FULLSCREEN);
            field.setFilters(new InputFilter[] {new InputFilter.LengthFilter(Math.max(1, maxLen))});
            field.setText(initial == null ? "" : initial);
            field.setSelection(field.getText().length());

            int pad = Math.round(20 * getResources().getDisplayMetrics().density);
            FrameLayout box = new FrameLayout(this);
            box.setPadding(pad, pad / 2, pad, 0);
            box.addView(field);

            // Set by OK or Done/Enter just before the dismiss; still null at the dismiss = cancelled.
            final String[] typed = {null};
            final AlertDialog dialog = new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                    .setTitle(label)
                    .setView(box)
                    .setPositiveButton(android.R.string.ok, new DialogInterface.OnClickListener() {
                        @Override
                        public void onClick(DialogInterface d, int which) {
                            typed[0] = field.getText().toString();
                        }
                    })
                    .setNegativeButton(android.R.string.cancel, null)
                    .create();
            dialog.setCanceledOnTouchOutside(true);
            dialog.setOnDismissListener(new DialogInterface.OnDismissListener() {
                @Override
                public void onDismiss(DialogInterface d) {
                    if (promptDialog == dialog) {
                        promptDialog = null;
                    }
                    finishPrompt(serial, typed[0]);
                }
            });
            field.setOnEditorActionListener(new TextView.OnEditorActionListener() {
                @Override
                public boolean onEditorAction(TextView v, int actionId, KeyEvent event) {
                    if (event == null) {
                        // The soft keyboard's Done key.
                        if (actionId != EditorInfo.IME_ACTION_DONE) {
                            return false;
                        }
                    } else if (event.getKeyCode() == KeyEvent.KEYCODE_ENTER
                            || event.getKeyCode() == KeyEvent.KEYCODE_NUMPAD_ENTER) {
                        // A hardware Enter: take the down, accept on the up, so no
                        // stray key-up reaches the game once the dialog has gone.
                        if (event.getAction() != KeyEvent.ACTION_UP) {
                            return true;
                        }
                    } else {
                        return false;
                    }
                    typed[0] = field.getText().toString();
                    dialog.dismiss();
                    return true;
                }
            });
            Window window = dialog.getWindow();
            if (window != null) {
                // Keyboard up as soon as the dialog has focus, and again after Home and return.
                window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_VISIBLE);
            }
            promptDialog = dialog;
            dialog.show();
            field.requestFocus();
            field.post(new Runnable() {
                @Override
                public void run() {
                    InputMethodManager imm = getSystemService(InputMethodManager.class);
                    if (imm != null) {
                        imm.showSoftInput(field, 0);
                    }
                }
            });
        } catch (RuntimeException e) {
            promptDialog = null;
            finishPrompt(serial, null);
        }
    }

    // ── The end of the process ──────────────────────────────────────────────────

    /** Whether onDestroy has started. Polled by the game a few times a second. */
    public boolean isBeingDestroyed() {
        return destroying;
    }

    /** The game has ended: its loop returned after its save, or it panicked. */
    public void reportGameEnded() {
        synchronized (endLock) {
            gameEnded = true;
            endLock.notifyAll();
        }
    }

    /**
     * Ends the process, so the next launch is a cold start: the game's event loop can be
     * built once per process (winit), so a second android_main in this process would fail.
     *
     * The order matters. The game sees `destroying`, saves a live world through its exit
     * function, ends its loop and reports; only then is the process killed. The kill comes
     * BEFORE super.onDestroy(): NativeActivity's onDestroy tears down the window and the input
     * queue, each a wait on the game's loop, and then waits for android_main to return, which
     * winit's loop never does on its own. After the bounded wait the process ends anyway.
     *
     * Process.killProcess (SIGKILL), not System.exit: it runs no atexit handlers, C++ static
     * destructors or Java shutdown hooks while the game's audio, GPU and network threads are
     * still running, the same end as any cached process the system reclaims.
     */
    @Override
    protected void onDestroy() {
        destroying = true;
        long deadline = SystemClock.uptimeMillis() + GAME_END_WAIT_MS;
        synchronized (endLock) {
            long left = GAME_END_WAIT_MS;
            while (!gameEnded && left > 0) {
                try {
                    endLock.wait(left);
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                    break;
                }
                left = deadline - SystemClock.uptimeMillis();
            }
        }
        Process.killProcess(Process.myPid());
        super.onDestroy();
    }
}

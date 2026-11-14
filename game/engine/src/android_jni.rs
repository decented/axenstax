//! Android only: `AxenstaxActivity`'s own methods, called over JNI: the text
//! prompt dialog (`text_prompt.rs`'s Android backend) and the end-of-process
//! handshake (`android_lifecycle.rs`).
//!
//! Every call is an instance method on the activity object android-activity hands
//! `ndk-context`, as `android_intent.rs` reads the intent: no `FindClass` by name
//! from this thread (it would search the system class loader, which cannot see
//! our class) and no Java `native` methods. None of them waits for the UI thread:
//! the dialog is posted with `runOnUiThread` and then polled, the destroy flag is
//! a volatile read, and the rest take a lock the UI thread only ever holds for a
//! few field writes.
#![cfg(target_os = "android")]

use std::sync::atomic::{AtomicBool, Ordering};

use jni::JNIEnv;
use jni::objects::{JObject, JString, JValue};

use crate::text_prompt::{PromptBackend, PromptKind, PromptPoll};

/// `AxenstaxActivity.textPromptState()` values.
const PROMPT_OPEN: i32 = 1;
const PROMPT_DONE: i32 = 2;

/// Say once per call site that a call failed (some are polled every frame).
fn warn_once(flag: &AtomicBool, what: &str, why: &dyn std::fmt::Display) {
    if !flag.swap(true, Ordering::Relaxed) {
        log::warn!("android: {what} failed ({why})");
    }
}

/// Run `call` against the activity in a local frame. A Java exception is cleared,
/// an error logged once (`warned`) and a panic caught: `None` then.
fn with_activity<T>(
    warned: &AtomicBool,
    what: &str,
    call: impl FnOnce(&mut JNIEnv<'_>, &JObject<'_>) -> jni::errors::Result<T>,
) -> Option<T> {
    let run = std::panic::AssertUnwindSafe(|| -> Option<T> {
        let Some((vm, activity)) = crate::android_intent::vm_and_activity() else {
            warn_once(warned, what, &"no JVM or activity yet");
            return None;
        };
        let mut env = match vm.attach_current_thread() {
            Ok(env) => env,
            Err(e) => {
                warn_once(warned, what, &e);
                return None;
            }
        };
        let out = env.with_local_frame(16, |env| -> jni::errors::Result<T> {
            // SAFETY: android-activity keeps the activity reference alive while
            // `android_main` runs, which is the only time these calls are made.
            let act = unsafe { JObject::from_raw(activity.cast()) };
            call(env, &act)
        });
        match out {
            Ok(v) => Some(v),
            Err(e) => {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                warn_once(warned, what, &e);
                None
            }
        }
    });
    std::panic::catch_unwind(run).unwrap_or_else(|_| {
        warn_once(warned, what, &"panicked");
        None
    })
}

/// The Android text prompt: `AxenstaxActivity.showTextPrompt` opens a dark
/// AlertDialog with a one-line EditText on the UI thread; its dismiss listener
/// (OK, Done/Enter, Cancel, Back or a tap outside) is the one place that posts
/// the answer, into a synchronized field polled here.
pub(crate) struct ActivityPrompt;

impl PromptBackend for ActivityPrompt {
    fn show(&mut self, label: &str, initial: &str, kind: PromptKind, max_chars: usize) -> bool {
        static WARNED: AtomicBool = AtomicBool::new(false);
        let max = i32::try_from(max_chars).unwrap_or(i32::MAX);
        with_activity(&WARNED, "showing the text prompt", |env, act| {
            let label = env.new_string(label)?;
            let initial = env.new_string(initial)?;
            env.call_method(
                act,
                "showTextPrompt",
                "(Ljava/lang/String;Ljava/lang/String;II)V",
                &[
                    JValue::Object(&*label),
                    JValue::Object(&*initial),
                    JValue::Int(kind.code()),
                    JValue::Int(max),
                ],
            )?;
            Ok(())
        })
        .is_some()
    }

    fn poll(&mut self) -> PromptPoll {
        static WARNED: AtomicBool = AtomicBool::new(false);
        let read = with_activity(&WARNED, "reading the text prompt", |env, act| {
            let state = env.call_method(act, "textPromptState", "()I", &[])?.i()?;
            if state != PROMPT_DONE {
                return Ok((state, None));
            }
            let answer = env.call_method(act, "takeTextPromptAnswer", "()Ljava/lang/String;", &[])?.l()?;
            if answer.is_null() {
                return Ok((state, None));
            }
            let answer = JString::from(answer);
            let text: String = env.get_string(&answer)?.into();
            Ok((state, Some(text)))
        });
        match read {
            Some((PROMPT_OPEN, _)) => PromptPoll::Waiting,
            Some((PROMPT_DONE, answer)) => PromptPoll::Answered(answer),
            // No prompt the activity knows of, or the call failed: a cancel,
            // never a keyboard that stays dead until restart.
            _ => PromptPoll::Gone,
        }
    }
}

/// Whether `AxenstaxActivity.onDestroy` has started (its volatile flag). A failed
/// read counts as "no".
pub(crate) fn activity_destroying() -> bool {
    static WARNED: AtomicBool = AtomicBool::new(false);
    with_activity(&WARNED, "reading the destroy flag", |env, act| {
        env.call_method(act, "isBeingDestroyed", "()Z", &[])?.z()
    })
    .unwrap_or(false)
}

/// Tell the activity the game has ended (its loop returned after the exit
/// function's save, or it panicked): its onDestroy may end the process now.
pub(crate) fn report_game_ended() {
    static WARNED: AtomicBool = AtomicBool::new(false);
    let told = with_activity(&WARNED, "reporting the game's end", |env, act| {
        env.call_method(act, "reportGameEnded", "()V", &[])?;
        Ok(())
    });
    if told.is_some() {
        log::info!("Android: game ended; the activity may end the process");
    }
}

//! Android only: the link a tap handed the game, read from the Activity's intent over JNI.
//!
//! `AxenstaxActivity` (tools/packaging/android/java) keeps the newest intent (`onNewIntent`
//! calls `setIntent`), so `getIntent().getDataString()` is the `axenstax:` link the player
//! last tapped. It is read here and consumed with `setData(null)`, so the same link is not
//! handled twice. What the game does with the text is `launch_links.rs`'s.
#![cfg(target_os = "android")]

use std::sync::atomic::{AtomicBool, Ordering};

use jni::objects::{JObject, JString, JValue};
use jni::{JNIEnv, JavaVM};

use crate::launch_links::IntentRead;

/// The Activity this process runs: android-activity's glue hands `ndk-context` the VM and the
/// NativeActivity instance before `android_main` starts.
fn vm_and_activity() -> Option<(JavaVM, *mut std::ffi::c_void)> {
    let ctx = std::panic::catch_unwind(ndk_context::android_context).ok()?;
    if ctx.vm().is_null() || ctx.context().is_null() {
        return None;
    }
    // SAFETY: the pointer is the process JavaVM, set up by android-activity.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.ok()?;
    Some((vm, ctx.context()))
}

/// Say once that the intent cannot be read (the poll repeats several times a second).
fn warn_once(what: &str) {
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        log::warn!("android: launch links: {what}");
    }
}

/// A Java `String` (or null) as a Rust string.
fn rust_string(env: &mut JNIEnv, obj: JObject) -> jni::errors::Result<Option<String>> {
    if obj.is_null() {
        return Ok(None);
    }
    let s = JString::from(obj);
    let out: String = env.get_string(&s)?.into();
    Ok(Some(out))
}

/// Read `getIntent()`'s data and consume it. A Java exception is cleared and a panic caught: a
/// link that cannot be read must never take the game down. The `catch_unwind` is live: the engine
/// sets no `panic = "abort"` (not in `Cargo.toml`, nor in `build-apk.sh`'s `RUSTFLAGS`), so a panic
/// here unwinds to it and the poll reports "unavailable"; without it, android-activity's guard
/// around `android_main` would abort the process.
pub fn read_launch_link() -> IntentRead {
    let run = std::panic::AssertUnwindSafe(|| -> IntentRead {
        let Some((vm, activity)) = vm_and_activity() else {
            warn_once("no JVM or activity yet");
            return IntentRead::Unavailable;
        };
        let mut env = match vm.attach_current_thread() {
            Ok(env) => env,
            Err(e) => {
                warn_once(&format!("no JNI env ({e})"));
                return IntentRead::Unavailable;
            }
        };
        let read = env.with_local_frame(16, |env| -> jni::errors::Result<IntentRead> {
            // SAFETY: android-activity keeps the activity reference alive while the loop runs.
            let act = unsafe { JObject::from_raw(activity.cast()) };
            let intent = env.call_method(&act, "getIntent", "()Landroid/content/Intent;", &[])?.l()?;
            if intent.is_null() {
                return Ok(IntentRead::NoLink);
            }
            let data = env.call_method(&intent, "getDataString", "()Ljava/lang/String;", &[])?.l()?;
            let Some(text) = rust_string(env, data)? else { return Ok(IntentRead::NoLink) };
            let consumed = match env.call_method(
                &intent,
                "setData",
                "(Landroid/net/Uri;)Landroid/content/Intent;",
                &[JValue::Object(&JObject::null())],
            ) {
                Ok(_) => true,
                Err(e) => {
                    let _ = env.exception_clear();
                    log::warn!("android: couldn't consume the launch intent: {e}");
                    false
                }
            };
            Ok(IntentRead::Link { text, consumed })
        });
        match read {
            Ok(read) => read,
            Err(e) => {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                warn_once(&format!("reading the intent failed ({e})"));
                IntentRead::Unavailable
            }
        }
    });
    std::panic::catch_unwind(run).unwrap_or_else(|_| {
        warn_once("reading the intent panicked");
        IntentRead::Unavailable
    })
}

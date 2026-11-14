//! Android: the game's process ends once the game has ended.
//!
//! winit builds ONE event loop per process (`EventLoopBuilder::build` answers
//! `RecreationAttempt` the second time), but an Android process outlives its
//! activity: Back at the lobby finished the activity, the process stayed cached,
//! and the next launch ran a second `android_main` in it, which panicked on that
//! error and left a blank splash. So the process ends with the game, after the
//! game has saved, and every launch is a cold start.
//!
//! Who does what:
//! - **The native side ends the process** ([`EndOnDrop`], the first local of
//!   `android_main`, so the last thing it drops): once the loop has returned and the
//!   game state has dropped, which is after the exit function's save; when the event
//!   loop could not be built; or while a panic unwinds. It is SIGKILL to our own pid
//!   ([`end_process`]), what `Process.killProcess` sends, with no JNI, so it also works
//!   when JNI or the activity is what failed.
//! - **The game ends** in one of two ways. Lobby Quit or Back runs the exit function
//!   and ends the loop. Or `AxenstaxActivity.onDestroy` raises a flag and carries
//!   straight on into `super.onDestroy()`; [`AndroidShell`] wraps the game's `App`,
//!   polls that flag, and when it is up runs the game's one exit function
//!   (`GameState::exit_app`: a live world is saved like Save & Quit) and ends the
//!   loop ([`on_destroy_signal`]).
//! - **Nothing ends the process on a timer.** The activity never ends it: a slow save
//!   holds the UI thread (in NativeActivity's teardown waits) for as long as it
//!   takes, and the process ends when the game does.
//!
//! Why from the native side, before `android_main` returns: android-activity 0.6's
//! UI-thread callbacks each wait for the native loop (`set_activity_state` for
//! pause/stop, `set_window` and `set_input` for the window and input-queue teardown,
//! and `notify_destroyed`, which waits for `android_main` to return), and winit 0.30
//! never ends its loop on Destroy (it logs a TODO). Once the loop has stopped, such a
//! wait can only end with the process. Ending it from the native side as soon as the
//! game is done resolves every one of them, whichever callback got there, without
//! depending on the UI thread reaching `onDestroy`. After `android_main` the glue
//! would only ask Java to finish the activity, detach the thread, release the
//! context and wake the UI thread: nothing a dead process needs.
//!
//! Home (and the screen turning off) is pause, stop and window termination,
//! never Destroy: the flag stays down, [`on_destroy_signal`] says
//! [`DestroyStep::Stay`], and the same process carries on when the player
//! comes back.

use std::time::Duration;
#[cfg(target_os = "android")]
use std::time::Instant;

#[cfg(target_os = "android")]
use winit::application::ApplicationHandler;

/// How often the shell asks the activity whether it is being destroyed. One
/// cheap JNI read; the UI thread sits in NativeActivity's teardown until the
/// game ends, so a quarter second adds little to that.
pub(crate) const DESTROY_POLL: Duration = Duration::from_millis(250);

/// What the shell does after a look at the activity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DestroyStep {
    /// Not being destroyed (Home, screen off, a dialog on top): carry on.
    Stay,
    /// Being destroyed: the exit function has run (`close_attempts` times);
    /// end the loop now.
    EndLoop { close_attempts: u8 },
}

/// The decision, kept pure for its test. `close` is the game's exit function
/// (`GameState::exit_app`), which saves a live world and returns whether the
/// app may exit. A close whose save failed normally keeps the window open to
/// retry; the activity is going away, so it is tried once more, and the second
/// close quits either way, keeping the autosave (`world_exit::close_window`).
pub(crate) fn on_destroy_signal(destroying: bool, mut close: impl FnMut() -> bool) -> DestroyStep {
    if !destroying {
        return DestroyStep::Stay;
    }
    if close() {
        return DestroyStep::EndLoop { close_attempts: 1 };
    }
    let _ = close();
    DestroyStep::EndLoop { close_attempts: 2 }
}

/// Whether the next look at the activity is due.
pub(crate) fn destroy_poll_due(last: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    last.is_none_or(|t| now.saturating_duration_since(t) >= DESTROY_POLL)
}

/// The game's `App`, run inside a shell that ends the loop when the activity is
/// being destroyed. Every `ApplicationHandler` call is passed straight to the
/// `App` (including the ones it leaves at winit's defaults, so nothing changes
/// if it grows one); only `about_to_wait`, the last call of every loop
/// iteration (winit sends it even while backgrounded), adds the look.
#[cfg(target_os = "android")]
pub(crate) struct AndroidShell<'a> {
    app: &'a mut crate::App,
    last_poll: Option<Instant>,
}

#[cfg(target_os = "android")]
impl<'a> AndroidShell<'a> {
    pub(crate) fn new(app: &'a mut crate::App) -> Self {
        Self { app, last_poll: None }
    }

    fn watch_destroy(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let now = Instant::now();
        if !destroy_poll_due(self.last_poll, now) {
            return;
        }
        self.last_poll = Some(now);
        let destroying = crate::android_jni::activity_destroying();
        let state = &mut self.app.state;
        let step = on_destroy_signal(destroying, || state.as_mut().is_none_or(|s| s.exit_app()));
        if let DestroyStep::EndLoop { close_attempts } = step {
            log::info!("Android: the activity is being destroyed; closed ({close_attempts}x), ending the loop");
            event_loop.exit();
        }
    }
}

#[cfg(target_os = "android")]
impl ApplicationHandler for AndroidShell<'_> {
    fn new_events(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, cause: winit::event::StartCause) {
        self.app.new_events(event_loop, cause);
    }

    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.app.resumed(event_loop);
    }

    fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: ()) {
        self.app.user_event(event_loop, event);
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        self.app.window_event(event_loop, window_id, event);
    }

    fn device_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        self.app.device_event(event_loop, device_id, event);
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.app.about_to_wait(event_loop);
        self.watch_destroy(event_loop);
    }

    fn suspended(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.app.suspended(event_loop);
    }

    fn exiting(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.app.exiting(event_loop);
    }

    fn memory_warning(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.app.memory_warning(event_loop);
    }
}

/// Runs `end` when dropped. `android_main` holds one with [`end_process`] as its
/// first local, so it drops last, however the function ends: after the loop has
/// returned and the game state has dropped (the exit function has saved a live
/// world), when the event loop could not be built, or while a panic unwinds out of
/// it.
pub(crate) struct EndOnDrop<F: FnOnce()> {
    end: Option<F>,
}

impl<F: FnOnce()> EndOnDrop<F> {
    pub(crate) fn new(end: F) -> Self {
        Self { end: Some(end) }
    }
}

impl<F: FnOnce()> Drop for EndOnDrop<F> {
    fn drop(&mut self) {
        if let Some(end) = self.end.take() {
            end();
        }
    }
}

/// End this process now: SIGKILL to our own pid, the signal
/// `Process.killProcess(Process.myPid())` sends, without a JNI call. No atexit
/// handlers or C++ static destructors run (`std::process::exit` would run them in
/// oboe, the Vulkan driver and libc++ while the audio, GPU and network threads
/// still do); what the save wrote is in the kernel's page cache and survives.
#[cfg(target_os = "android")]
pub(crate) fn end_process() {
    log::info!("Android: the game has ended; ending the process, so the next launch is a cold start");
    // SAFETY: kill(2) on our own pid has no memory-safety preconditions.
    unsafe {
        libc::kill(libc::getpid(), libc::SIGKILL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::time::Instant;

    #[test]
    fn home_and_return_never_close_the_game() {
        let mut closes = 0;
        let step = on_destroy_signal(false, || {
            closes += 1;
            true
        });
        assert_eq!(step, DestroyStep::Stay);
        assert_eq!(closes, 0, "no save, no exit: the same process carries on");
    }

    #[test]
    fn destroy_closes_the_game_before_the_loop_ends() {
        let mut order = Vec::new();
        let step = on_destroy_signal(true, || {
            order.push("close (saves a live world)");
            true
        });
        order.push("loop ends");
        assert_eq!(step, DestroyStep::EndLoop { close_attempts: 1 });
        assert_eq!(order, ["close (saves a live world)", "loop ends"]);
    }

    #[test]
    fn a_failed_close_save_is_tried_once_more_and_the_loop_still_ends() {
        let mut results = vec![false, true].into_iter();
        let mut closes = 0;
        let step = on_destroy_signal(true, || {
            closes += 1;
            results.next().unwrap()
        });
        assert_eq!(step, DestroyStep::EndLoop { close_attempts: 2 });
        assert_eq!(closes, 2);

        // Even if the second close also says "stay", the activity is going:
        // the loop ends (that close has already quit keeping the autosave).
        let step = on_destroy_signal(true, || false);
        assert_eq!(step, DestroyStep::EndLoop { close_attempts: 2 });
    }

    /// Pushes its name onto the log when dropped: the game state, in tests.
    struct Dropped<'a>(&'a RefCell<Vec<&'static str>>, &'static str);

    impl Drop for Dropped<'_> {
        fn drop(&mut self) {
            self.0.borrow_mut().push(self.1);
        }
    }

    #[test]
    fn the_process_ends_only_after_the_game_has_ended() {
        // `android_main`'s shape: the guard first, then the game, whose loop ends
        // on the destroy flag after its close (the save).
        let log = RefCell::new(Vec::new());
        {
            let _end = EndOnDrop::new(|| log.borrow_mut().push("process ends"));
            let _game = Dropped(&log, "game state dropped");
            let step = on_destroy_signal(true, || {
                log.borrow_mut().push("close (saves a live world)");
                true
            });
            assert_eq!(step, DestroyStep::EndLoop { close_attempts: 1 });
            log.borrow_mut().push("loop returned");
            assert!(!log.borrow().contains(&"process ends"), "never while the game is alive");
        }
        assert_eq!(
            *log.borrow(),
            ["close (saves a live world)", "loop returned", "game state dropped", "process ends"]
        );
    }

    #[test]
    fn a_game_that_panics_still_ends_the_process_after_its_state_dropped() {
        let log = RefCell::new(Vec::new());
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _end = EndOnDrop::new(|| log.borrow_mut().push("process ends"));
            let _game = Dropped(&log, "game state dropped");
            panic!("the game panicked (expected by this test)");
        }));
        assert!(unwound.is_err());
        assert_eq!(*log.borrow(), ["game state dropped", "process ends"]);
    }

    /// The activity's Java with its comments taken out, so prose can't satisfy or
    /// break the checks below.
    fn activity_code() -> String {
        let src = include_str!("../../../tools/packaging/android/java/com/axenstax/game/AxenstaxActivity.java");
        let mut code = String::new();
        let mut rest = src;
        loop {
            let line = rest.find("//");
            let block = rest.find("/*");
            match (line, block) {
                (None, None) => {
                    code.push_str(rest);
                    return code;
                }
                (Some(l), b) if b.is_none_or(|b| l < b) => {
                    code.push_str(&rest[..l]);
                    rest = rest[l..].find('\n').map_or("", |n| &rest[l + n..]);
                }
                (_, Some(b)) => {
                    code.push_str(&rest[..b]);
                    rest = rest[b..].find("*/").map_or("", |e| &rest[b + e + 2..]);
                }
                (Some(_), None) => unreachable!(),
            }
        }
    }

    /// The statements of the method whose body follows `signature`.
    fn method_statements(code: &str, signature: &str) -> Vec<String> {
        let at = code.find(signature).unwrap_or_else(|| panic!("no `{signature}` in the activity"));
        let open = at + code[at..].find('{').expect("a method body");
        let mut depth = 0;
        for (i, c) in code[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return code[open + 1..open + i]
                            .split(';')
                            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
                            .filter(|s| !s.is_empty())
                            .collect();
                    }
                }
                _ => {}
            }
        }
        panic!("`{signature}` never closes");
    }

    #[test]
    fn the_activity_raises_the_flag_and_never_ends_the_process_on_a_timer() {
        // Pinned here because the Java has no tests of its own. onDestroy says the
        // activity is going and carries on: no wait, no timer, no kill. The process
        // ends only from `EndOnDrop`, after the game has ended, so a slow save is
        // never cut short.
        let code = activity_code();
        assert_eq!(
            method_statements(&code, "protected void onDestroy()"),
            ["destroying = true", "super.onDestroy()"]
        );
        for never in ["killProcess", "sendSignal", "System.exit", "Runtime.getRuntime", ".wait("] {
            assert!(!code.contains(never), "AxenstaxActivity must not use `{never}`");
        }
    }

    #[test]
    fn the_activity_is_asked_at_most_every_poll_interval() {
        let t0 = Instant::now();
        assert!(destroy_poll_due(None, t0), "the first look is immediate");
        assert!(!destroy_poll_due(Some(t0), t0));
        assert!(!destroy_poll_due(Some(t0), t0 + DESTROY_POLL / 2));
        assert!(destroy_poll_due(Some(t0), t0 + DESTROY_POLL));
    }
}

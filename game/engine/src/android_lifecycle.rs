//! Android: the game's process ends once its activity is destroyed.
//!
//! winit builds ONE event loop per process (`EventLoopBuilder::build` answers
//! `RecreationAttempt` the second time), but an Android process outlives its
//! activity: Back at the lobby finished the activity, the process stayed cached,
//! and the next launch ran a second `android_main` in it, which panicked on that
//! error and left a blank splash. So once the activity is destroyed, finishing
//! or not, the process ends, after the game has saved, and every launch is a
//! cold start.
//!
//! Who does what:
//! - `AxenstaxActivity.onDestroy` (tools/packaging/android/java) raises a flag
//!   first thing, waits (bounded) for the game to report that it has ended, then
//!   kills the process, BEFORE `super.onDestroy()`.
//! - [`AndroidShell`] wraps the game's `App` and polls that flag. When it is up,
//!   it runs the game's one exit function (`GameState::exit_app`: a live world
//!   is saved like Save & Quit) and ends the loop ([`on_destroy_signal`]).
//! - `android_main` reports "ended" once the loop has returned and the game
//!   state is dropped (a drop guard, so a panic reports too).
//!
//! Why the kill comes before `super.onDestroy()`: android-activity's onDestroy
//! waits for `android_main` to return (`notify_destroyed` in its glue), but
//! winit 0.30 does not end its loop on Destroy (it logs a TODO), and
//! `NativeActivity.onDestroy` first tears down the window and the input queue,
//! each a wait on the native loop. A loop that ended on the flag would leave the
//! UI thread waiting on a loop that no longer runs; a loop that never ended
//! would leave it waiting forever. Killing before that teardown has neither
//! problem, and the wait for "ended" puts it after the save.
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
/// cheap JNI read; a quarter second is quick enough against the activity's
/// bounded wait, which covers it plus the save.
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

/// Reports "the game has ended" to the activity when dropped: at the end of
/// `android_main`, after the loop returned and the game state was dropped, or
/// while a panic unwinds out of it. The activity's onDestroy waits for this
/// before ending the process.
#[cfg(target_os = "android")]
pub(crate) struct ReportEndOnDrop;

#[cfg(target_os = "android")]
impl Drop for ReportEndOnDrop {
    fn drop(&mut self) {
        crate::android_jni::report_game_ended();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn the_activity_is_asked_at_most_every_poll_interval() {
        let t0 = Instant::now();
        assert!(destroy_poll_due(None, t0), "the first look is immediate");
        assert!(!destroy_poll_due(Some(t0), t0));
        assert!(!destroy_poll_due(Some(t0), t0 + DESTROY_POLL / 2));
        assert!(destroy_poll_due(Some(t0), t0 + DESTROY_POLL));
    }
}

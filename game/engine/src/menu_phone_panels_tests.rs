//! The Settings panel and the pause menu on a phone, drawn headless (egui is pure CPU): on a
//! compact screen every button that leaves them (Back, Resume, Save, Save and Quit, Quit
//! Without Saving, the confirm prompts, a Trial's buttons) is on the screen without
//! scrolling, and the rest of each panel is reachable by scrolling. A tall window keeps the
//! old layout.
use super::keeps_dialog_tests::{placed, texts};
use super::*;

const PIXEL_PORTRAIT: egui::Vec2 = egui::vec2(412.0, 915.0);
const SMALL_PORTRAIT: egui::Vec2 = egui::vec2(360.0, 640.0);
const PIXEL_LANDSCAPE: egui::Vec2 = egui::vec2(914.0, 411.0);
const SMALL_LANDSCAPE: egui::Vec2 = egui::vec2(640.0, 360.0);
const PHONES: [egui::Vec2; 4] = [PIXEL_PORTRAIT, SMALL_PORTRAIT, PIXEL_LANDSCAPE, SMALL_LANDSCAPE];
const DESKTOP: egui::Vec2 = egui::vec2(1280.0, 900.0);

/// The compact layout, forced on: on Android it is always on, whatever the height (a Pixel in
/// portrait is 412 x 915 points), but a Linux test only gets it under 620 points tall.
fn force_compact(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("test-force-compact-dialog"), on));
}

fn screen_of(size: egui::Vec2) -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, size)
}

/// Drawn whole, inside the visible area of whatever clips it, inside the screen.
fn on_screen(output: &egui::FullOutput, size: egui::Vec2, label: &str) -> bool {
    match placed(output, label) {
        Some((text, clip)) => clip.contains_rect(text) && screen_of(size).contains_rect(text) && screen_of(size).contains_rect(clip),
        None => false,
    }
}

fn wheel(at: egui::Pos2, dy: f32) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(at),
        egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, dy), phase: egui::TouchPhase::Move, modifiers: Default::default() },
    ]
}

fn tap_events(at: egui::Pos2) -> [Vec<egui::Event>; 3] {
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    [vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)]]
}

// ---- Settings ------------------------------------------------------------------------------

struct SettingsRig {
    ctx: egui::Context,
    size: egui::Vec2,
    state: SettingsPanelState,
    gfx: crate::graphics_settings::GraphicsSettings,
    action: SettingsPanelAction,
    output: egui::FullOutput,
}

impl SettingsRig {
    fn new(size: egui::Vec2, compact: Option<bool>) -> SettingsRig {
        let ctx = egui::Context::default();
        if let Some(on) = compact {
            force_compact(&ctx, on);
        }
        let mut rig = SettingsRig { ctx, size, state: SettingsPanelState::default(), gfx: Default::default(), action: SettingsPanelAction::None, output: Default::default() };
        for _ in 0..6 {
            rig.frame(vec![]);
        }
        rig
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let input = egui::RawInput { screen_rect: Some(screen_of(self.size)), events, ..Default::default() };
        let (state, gfx, action) = (&mut self.state, &mut self.gfx, &mut self.action);
        self.output = self.ctx.run(input, |ctx| {
            *action = draw_settings_panel(ctx, state, gfx);
        });
    }

    fn on_screen(&self, label: &str) -> bool {
        on_screen(&self.output, self.size, label)
    }

    /// The Window's own rectangle.
    fn window(&self) -> egui::Rect {
        self.ctx.memory(|m| m.area_rect(egui::Id::new("Graphics"))).expect("the Settings window was not drawn")
    }

    fn scroll_to_end(&mut self) {
        let at = self.size.to_pos2() * 0.5;
        for _ in 0..40 {
            self.frame(wheel(at, -400.0));
        }
        self.frame(vec![]);
    }

    fn tap(&mut self, label: &str) {
        let at = placed(&self.output, label).unwrap_or_else(|| panic!("no {label:?} on screen: {:?}", texts(&self.output))).0.center();
        for events in tap_events(at) {
            self.frame(events);
        }
        self.frame(vec![]);
    }
}

#[test]
fn settings_back_is_on_screen_with_no_scrolling_on_every_phone_size() {
    for size in PHONES {
        let rig = SettingsRig::new(size, Some(true));
        assert!(rig.on_screen("Back"), "Back is not on the {size:?} screen: {:?}", texts(&rig.output));
        assert!(rig.on_screen("Graphics"), "the title is not on the {size:?} screen");
        assert!(rig.on_screen(&format!("Preset: {}", rig.gfx.preset().label())), "the first line is not on screen at {size:?}");
        let window = rig.window();
        assert!(screen_of(size).contains_rect(window), "the Settings window {window:?} is not inside the {size:?} screen");
    }
}

#[test]
fn the_last_setting_scrolls_into_view_on_every_phone_size() {
    let last = crate::tester_gate::version_line();
    for size in PHONES {
        let mut rig = SettingsRig::new(size, Some(true));
        assert!(!rig.on_screen(&last), "the last line is on the {size:?} screen before any scrolling: nothing would scroll");
        rig.scroll_to_end();
        assert!(rig.on_screen(&last), "the last line did not scroll into view at {size:?}: {:?}", texts(&rig.output));
        assert!(rig.on_screen("Back"), "Back left the screen when scrolled at {size:?}");
        assert!(screen_of(size).contains_rect(rig.window()), "the window left the {size:?} screen when scrolled");
    }
}

#[test]
fn the_pinned_back_closes_the_panel_on_a_phone() {
    let mut rig = SettingsRig::new(SMALL_PORTRAIT, Some(true));
    rig.scroll_to_end();
    rig.action = SettingsPanelAction::None;
    let at = placed(&rig.output, "Back").unwrap().0.center();
    let mut seen = SettingsPanelAction::None;
    for events in tap_events(at) {
        rig.frame(events);
        if rig.action == SettingsPanelAction::Back {
            seen = SettingsPanelAction::Back;
        }
    }
    assert_eq!(seen, SettingsPanelAction::Back);
}

#[test]
fn a_dial_still_changes_in_the_scrolling_panel() {
    let mut rig = SettingsRig::new(PIXEL_PORTRAIT, Some(true));
    let before = rig.gfx.fog;
    rig.tap("Fog");
    assert_ne!(rig.gfx.fog, before, "tapping Fog in the scrolling panel changed nothing");
}

#[test]
fn a_tall_window_keeps_the_settings_panel_unscrolled() {
    // Forced off at a phone size, and a tall desktop window: no scroll area, the version
    // line is drawn at once (the panel is as tall as its content, as before).
    let last = crate::tester_gate::version_line();
    let rig = SettingsRig::new(egui::vec2(1280.0, 1400.0), None);
    assert!(rig.on_screen(&last) && rig.on_screen("Back"), "{:?}", texts(&rig.output));
    let content = SettingsRig::new(egui::vec2(1280.0, 1400.0), Some(false)).window();
    assert_eq!(rig.window(), content);
    let forced = SettingsRig::new(PIXEL_PORTRAIT, Some(false));
    assert!(forced.window().height() > PIXEL_PORTRAIT.y, "the uncompact panel should be taller than a phone: {:?}", forced.window());
}

// ---- Pause menu ----------------------------------------------------------------------------

struct PauseRig {
    ctx: egui::Context,
    size: egui::Vec2,
    confirm_quit: bool,
    confirm_creative: bool,
    difficulty: String,
    cloud_save: bool,
    is_trial: bool,
    clicked: i32,
    output: egui::FullOutput,
}

impl PauseRig {
    fn new(size: egui::Vec2, is_trial: bool) -> PauseRig {
        PauseRig::with(size, is_trial, true)
    }

    fn with(size: egui::Vec2, is_trial: bool, compact: bool) -> PauseRig {
        let ctx = egui::Context::default();
        force_compact(&ctx, compact);
        let mut rig = PauseRig { ctx, size, confirm_quit: false, confirm_creative: false, difficulty: "normal".into(), cloud_save: false, is_trial, clicked: -1, output: Default::default() };
        for _ in 0..4 {
            rig.frame(vec![]);
        }
        rig
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let input = egui::RawInput { screen_rect: Some(screen_of(self.size)), events, ..Default::default() };
        let mut clicked = -1;
        let (cq, cc, diff, cloud, trial) = (&mut self.confirm_quit, &mut self.confirm_creative, &mut self.difficulty, &mut self.cloud_save, self.is_trial);
        self.output = self.ctx.run(input, |ctx| {
            clicked = draw_pause_menu(ctx, "The Workshop", cq, cc, diff, false, false, 1, cloud, trial, None);
        });
        if clicked != -1 {
            self.clicked = clicked;
        }
    }

    fn on_screen(&self, label: &str) -> bool {
        on_screen(&self.output, self.size, label)
    }

    fn scroll_to_end(&mut self) {
        let at = egui::pos2(self.size.x * 0.5, self.size.y - 20.0);
        for _ in 0..40 {
            self.frame(wheel(at, -400.0));
        }
        self.frame(vec![]);
    }

    /// Swipe up a little at a time until `label` is on screen (a small scroll area shows one
    /// row or two at once).
    fn reveal(&mut self, label: &str) {
        let at = egui::pos2(self.size.x * 0.5, self.size.y - 20.0);
        for _ in 0..80 {
            // Let the scroll settle (egui eases it) before looking.
            for _ in 0..12 {
                self.frame(vec![]);
            }
            if self.on_screen(label) {
                return;
            }
            self.frame(wheel(at, -30.0));
        }
        panic!("{label:?} never scrolled into view at {:?}: {:?}", self.size, texts(&self.output));
    }

    fn tap(&mut self, label: &str) {
        let at = placed(&self.output, label).unwrap_or_else(|| panic!("no {label:?} on screen: {:?}", texts(&self.output))).0.center();
        self.clicked = -1;
        for events in tap_events(at) {
            self.frame(events);
        }
        self.frame(vec![]);
    }
}

fn leave_labels() -> [String; 4] {
    ["Resume".to_string(), "Save".to_string(), "Save and Quit".to_string(), crate::world_exit::quit_no_save_label(None)]
}

#[test]
fn every_leave_button_of_the_pause_menu_is_on_screen_with_no_scrolling_on_every_phone_size() {
    for size in PHONES {
        let rig = PauseRig::new(size, false);
        for label in leave_labels() {
            assert!(rig.on_screen(&label), "{label:?} is not on the {size:?} screen: {:?}", texts(&rig.output));
        }
        assert!(rig.on_screen("Paused"), "the title is not on the {size:?} screen");
    }
}

#[test]
fn the_rest_of_the_pause_menu_scrolls_into_view_on_every_phone_size() {
    for size in PHONES {
        let mut rig = PauseRig::new(size, false);
        rig.scroll_to_end();
        for label in leave_labels() {
            assert!(rig.on_screen(&label), "{label:?} left the {size:?} screen when the rest scrolled");
        }
        // The last control of a survival world's menu is at the end of the scroll.
        assert!(rig.on_screen("Switch to Creative"), "the last control did not scroll into view at {size:?}: {:?}", texts(&rig.output));
        // ...and every other control comes into view on the way down.
        for label in ["Controls", "Graphics", "Your look"] {
            let mut rig = PauseRig::new(size, false);
            rig.reveal(label);
        }
    }
}

#[test]
fn the_difficulty_row_is_on_screen_on_every_phone_size() {
    for size in PHONES {
        let rig = PauseRig::new(size, false);
        for label in ["Peaceful", "Easy", "Normal", "Hard"] {
            assert!(rig.on_screen(label), "{label:?} is not on the {size:?} screen: {:?}", texts(&rig.output));
        }
        // ...and the row is centred, not pushed against the screen's left edge.
        let x = |label: &str| placed(&rig.output, label).unwrap().0.center().x;
        let middle = (x("Peaceful") + x("Hard")) / 2.0;
        assert!((middle - size.x / 2.0).abs() < 20.0, "the difficulty row is not centred at {size:?}: its middle is at {middle}");
    }
}

#[test]
fn a_phone_taps_save_and_quit_and_resume() {
    for size in PHONES {
        let mut rig = PauseRig::new(size, false);
        rig.tap("Save and Quit");
        assert_eq!(rig.clicked, PAUSE_SAVE_QUIT, "Save and Quit at {size:?}");
        rig.tap("Resume");
        assert_eq!(rig.clicked, PAUSE_RESUME, "Resume at {size:?}");
        rig.reveal("Graphics");
        rig.tap("Graphics");
        assert_eq!(rig.clicked, PAUSE_OPEN_SETTINGS, "Graphics at {size:?}");
    }
}

#[test]
fn quit_without_saving_asks_first_and_the_prompt_is_on_screen_on_every_phone_size() {
    for size in PHONES {
        let mut rig = PauseRig::new(size, false);
        rig.tap(&crate::world_exit::quit_no_save_label(None));
        assert!(rig.confirm_quit, "tapping Quit Without Saving did not ask at {size:?}");
        for label in ["Yes, Quit", "Go Back"] {
            assert!(rig.on_screen(label), "{label:?} is not on the {size:?} screen: {:?}", texts(&rig.output));
        }
        rig.tap("Go Back");
        assert!(!rig.confirm_quit);
    }
}

#[test]
fn the_switch_to_creative_prompt_is_on_screen_on_every_phone_size() {
    for size in PHONES {
        let mut rig = PauseRig::new(size, false);
        rig.reveal("Switch to Creative");
        rig.tap("Switch to Creative");
        assert!(rig.confirm_creative, "tapping Switch to Creative did not ask at {size:?}");
        for label in ["Yes, Switch to Creative", "Go Back"] {
            assert!(rig.on_screen(label), "{label:?} is not on the {size:?} screen: {:?}", texts(&rig.output));
        }
    }
}

#[test]
fn a_trials_pause_menu_is_on_screen_on_every_phone_size() {
    for size in PHONES {
        let rig = PauseRig::new(size, true);
        for label in ["Resume", "\u{21bb} Try Again", "\u{2190} Leave", "Controls"] {
            assert!(rig.on_screen(label), "{label:?} is not on the {size:?} screen: {:?}", texts(&rig.output));
        }
    }
}

#[test]
fn a_tall_window_keeps_the_pause_menu_as_it_was() {
    let rig = PauseRig::with(DESKTOP, false, false);
    let y = |label: &str| placed(&rig.output, label).unwrap_or_else(|| panic!("no {label:?}")).0.center().y;
    // The title sits 100 points down, then Difficulty, Stash, the leave buttons, Controls...
    assert!(y("Paused") > 100.0);
    let order = ["Paused", "Difficulty", "Resume", "Save", "Save and Quit", "Controls", "Graphics", "Your look", "Switch to Creative"];
    for pair in order.windows(2) {
        assert!(y(pair[0]) < y(pair[1]), "{} is not above {}", pair[0], pair[1]);
    }
    let stash_y = texts(&rig.output).iter().find(|(t, _)| t.contains("Stash")).expect("no Stash button").1.y;
    assert!(y("Difficulty") < stash_y && stash_y < y("Resume"));
}

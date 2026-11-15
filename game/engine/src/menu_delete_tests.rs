//! The delete-world dialog, drawn headless through the lobby's own dialog arm: a two-button
//! confirm (Cancel, Delete), no typed name. Cancel has the keyboard focus, so Enter can only
//! cancel; Delete fires on a click or tap and on nothing else, so no run of Enters, Spaces or
//! Tabs can delete a world. Esc cancels; a click outside the card does nothing.
use super::keeps_dialog_tests::texts;
use super::*;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 900.0);
const FOLDER: &str = "doomed-world";
const NAME: &str = "Doomed World";

struct Rig {
    ctx: egui::Context,
    state: MenuState,
    _root: crate::save::WorldsRootGuard,
}

impl Rig {
    /// A lobby with one world on disk and its delete dialog open, settled for two frames.
    fn new(tag: &str) -> Rig {
        let root = crate::save::WorldsRootGuard::new(tag);
        let dir = crate::save::world_dir(FOLDER);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("world.dat"), b"precious").unwrap();
        let mut state = MenuState::new();
        state.worlds.push(crate::save::WorldEntry {
            folder_name: FOLDER.to_string(),
            meta: crate::save::WorldMeta::new(NAME),
            last_played: std::time::SystemTime::now(),
            size_bytes: 8,
            cloud_only: false,
        });
        state.dialog = delete_dialog(0);
        let mut rig = Rig { ctx: egui::Context::default(), state, _root: root };
        rig.frame(vec![]);
        rig.frame(vec![]);
        rig
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> (MenuAction, Vec<(String, egui::Pos2)>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)),
            events,
            ..Default::default()
        };
        let mut action = MenuAction::None;
        let state = &mut self.state;
        let output = self.ctx.run(input, |ctx| action = draw_main_menu(ctx, state));
        (action, texts(&output))
    }

    fn press(&mut self, key: egui::Key) -> MenuAction {
        let event = |pressed| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: Default::default(),
        };
        let (action, _) = self.frame(vec![event(true), event(false)]);
        self.frame(vec![]);
        action
    }

    fn tap(&mut self, at: egui::Pos2) -> MenuAction {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]).0
    }

    fn at(&mut self, label: &str) -> egui::Pos2 {
        let (_, seen) = self.frame(vec![]);
        seen.iter().find(|(t, _)| t == label).unwrap_or_else(|| panic!("no '{label}' drawn: {seen:?}")).1
    }

    fn world_exists(&self) -> bool {
        crate::save::world_dir(FOLDER).join("world.dat").exists()
    }

    fn dialog_open(&self) -> bool {
        matches!(self.state.dialog, MenuDialog::Delete { .. })
    }
}

fn delete_dialog(world_idx: usize) -> MenuDialog {
    MenuDialog::Delete { world_idx }
}

#[test]
fn the_dialog_asks_the_question_and_has_two_buttons_and_no_text_field() {
    let mut rig = Rig::new("delete-dialog-words");
    let (_, seen) = rig.frame(vec![]);
    let has = |t: &str| seen.iter().any(|(s, _)| s == t);
    assert!(has("Delete this world?"), "{seen:?}");
    // The dialog names the world as the list shows it (this rig writes no meta file, so the
    // list falls back to the folder name).
    let MenuDialog::Delete { world_idx } = rig.state.dialog else { panic!("dialog closed") };
    let listed = rig.state.worlds[world_idx].meta.display_name.clone();
    assert!(has(&format!("Are you sure you want to delete {listed}? This can't be undone.")), "{seen:?}");
    assert!(has("Cancel") && has("Delete"), "{seen:?}");
    assert!(!seen.iter().any(|(s, _)| s.contains("Type ") || s.contains("Delete Forever")), "{seen:?}");
    // Cancel on the left, Delete on the right.
    let (cancel, delete) = (rig.at("Cancel"), rig.at("Delete"));
    assert!(cancel.x < delete.x, "Cancel {cancel:?}, Delete {delete:?}");
    // And the pair sits centred under the title, not against the card's left edge.
    let title = rig.at("Delete this world?");
    let middle = (cancel.x + delete.x) / 2.0;
    assert!((middle - title.x).abs() < 12.0, "buttons centred at {middle}, title at {}", title.x);
}

#[test]
fn enter_pressed_five_times_never_deletes_the_world() {
    let mut rig = Rig::new("delete-dialog-enter");
    for _ in 0..5 {
        let action = rig.press(egui::Key::Enter);
        assert!(!matches!(action, MenuAction::RefreshWorlds), "a delete was run");
    }
    assert!(rig.world_exists(), "Enter deleted the world");
}

#[test]
fn enter_activates_cancel_which_holds_the_focus() {
    let mut rig = Rig::new("delete-dialog-focus");
    rig.press(egui::Key::Enter);
    assert!(!rig.dialog_open(), "Enter did not reach the focused Cancel");
    assert!(rig.world_exists());
}

#[test]
fn moving_the_focus_to_delete_and_pressing_enter_or_space_still_does_not_delete() {
    let mut rig = Rig::new("delete-dialog-tab");
    rig.press(egui::Key::Tab);
    for key in [egui::Key::Enter, egui::Key::Space, egui::Key::Enter] {
        let action = rig.press(key);
        assert!(!matches!(action, MenuAction::RefreshWorlds));
    }
    rig.press(egui::Key::ArrowRight);
    rig.press(egui::Key::Enter);
    assert!(rig.world_exists(), "a keyboard activation of Delete deleted the world");
}

#[test]
fn a_click_on_delete_deletes_the_world() {
    let mut rig = Rig::new("delete-dialog-click");
    let at = rig.at("Delete");
    let action = rig.tap(at);
    assert!(matches!(action, MenuAction::RefreshWorlds), "the lobby was not told to re-list");
    assert!(!rig.world_exists(), "the world is still on disk");
    assert!(matches!(rig.state.dialog, MenuDialog::None));
}

#[test]
fn a_click_on_cancel_keeps_the_world_and_closes_the_dialog() {
    let mut rig = Rig::new("delete-dialog-cancel");
    let at = rig.at("Cancel");
    rig.tap(at);
    assert!(rig.world_exists());
    assert!(!rig.dialog_open());
}

#[test]
fn escape_pressed_while_cancel_has_focus_closes_the_dialog_and_keeps_the_world() {
    // The real routing: egui sees the Escape first (the focused button swallows it), so the
    // lobby's own Escape handler never runs; the dialog has to read the key itself.
    let mut rig = Rig::new("delete-dialog-escape");
    rig.press(egui::Key::Escape);
    assert!(!rig.dialog_open(), "Escape did not close the dialog");
    assert!(rig.world_exists());
}

#[test]
fn escape_with_the_focus_moved_to_delete_cancels_and_never_deletes() {
    let mut rig = Rig::new("delete-dialog-escape-delete");
    rig.press(egui::Key::Tab);
    let action = rig.press(egui::Key::Escape);
    assert!(!matches!(action, MenuAction::RefreshWorlds));
    assert!(!rig.dialog_open());
    assert!(rig.world_exists());
}

#[test]
fn android_back_closes_the_dialog_and_keeps_the_world() {
    // winit reports Back as the logical BrowserBack key; egui sees that, not Escape.
    let mut rig = Rig::new("delete-dialog-back");
    rig.press(egui::Key::BrowserBack);
    assert!(!rig.dialog_open(), "Back did not close the dialog");
    assert!(rig.world_exists());
}

#[test]
fn a_click_outside_the_card_does_nothing() {
    let mut rig = Rig::new("delete-dialog-outside");
    rig.tap(egui::pos2(6.0, 6.0));
    assert!(rig.dialog_open(), "the click outside closed the dialog");
    assert!(rig.world_exists());
}

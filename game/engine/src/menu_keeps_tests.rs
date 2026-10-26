//! My Keeps under the text box of the Join Game dialog, drawn headless (egui is pure CPU, so
//! no GPU is needed): the saved Keeps are listed at every screen width, the block is absent
//! when there are none, Join hands back the Keep's invite with no confirm and closes the
//! dialog, and Forget takes two taps. Click positions come from the text each frame drew.
use super::*;
use crate::keeps::{Invite, FORGET_CONFIRM, MY_KEEPS};

const HOST: [u8; 32] = [0xab; 32];
const SECRET: [u8; 32] = [0x01; 32];

struct TempFile(std::path::PathBuf);

impl TempFile {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("axenstax-menu-keeps-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        TempFile(dir.join("keeps.json"))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if let Some(dir) = self.0.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// A saved-Keeps file holding one Keep named "Fray test host", joined just now.
fn saved(file: &TempFile) -> Vec<crate::keeps::Keep> {
    let invite = Invite { host_key: HOST, secret: SECRET };
    assert!(crate::keeps::save_invite_join(&file.0, &invite, Some("Fray test host"), crate::keeps::unix_now()));
    crate::keeps::load_list(&file.0)
}

/// Every piece of text a frame drew, with where its middle is.
fn texts(output: &egui::FullOutput) -> Vec<(String, egui::Pos2)> {
    fn walk(shape: &egui::epaint::Shape, out: &mut Vec<(String, egui::Pos2)>) {
        match shape {
            egui::epaint::Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos + t.galley.rect.center().to_vec2())),
            egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    output.shapes.iter().for_each(|clipped| walk(&clipped.shape, &mut out));
    out
}

fn has(seen: &[(String, egui::Pos2)], text: &str) -> bool {
    seen.iter().any(|(t, _)| t == text)
}

/// The Join Game dialog, a frame at a time, on a screen of `size` points.
struct Rig {
    ctx: egui::Context,
    size: egui::Vec2,
    keeps: Vec<crate::keeps::Keep>,
    armed: Option<String>,
    join: MenuAction,
    file: TempFile,
    seen: Vec<(String, egui::Pos2)>,
}

impl Rig {
    fn new(tag: &str, size: egui::Vec2, with_keep: bool) -> Rig {
        let file = TempFile::new(tag);
        let keeps = if with_keep { saved(&file) } else { Vec::new() };
        let mut rig = Rig { ctx: egui::Context::default(), size, keeps, armed: None, join: MenuAction::None, file, seen: Vec::new() };
        // egui lays a widget out one frame before it can be hit: draw twice before any click.
        rig.frame(vec![]);
        rig.frame(vec![]);
        rig
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let lan = crate::lan_ui::LanGames { rooms: vec![], listening: true };
        let mut picked = None;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            events,
            ..Default::default()
        };
        let (keeps, armed, join, file) = (&mut self.keeps, &mut self.armed, &mut self.join, &self.file.0);
        let output = self.ctx.run(input, |ctx| {
            let _ = draw_join_dialog(ctx, "", &lan, &mut picked, JoinKeepsPane { keeps, forget_armed: armed, file, join });
        });
        self.seen = texts(&output);
    }

    /// Tap the text `label` (a press in one frame, the release in the next, like a finger).
    fn tap(&mut self, label: &str) {
        let at = self.seen.iter().find(|(t, _)| t == label).unwrap_or_else(|| panic!("no {label:?} on screen: {:?}", self.seen)).1;
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.frame(vec![]);
    }
}

const DESKTOP: egui::Vec2 = egui::vec2(1280.0, 720.0);
const PHONE: egui::Vec2 = egui::vec2(914.0, 411.0);

#[test]
fn the_join_dialog_lists_the_saved_keeps_at_every_width() {
    for (name, size) in [("desktop", DESKTOP), ("phone", PHONE), ("narrow", egui::vec2(500.0, 411.0))] {
        let rig = Rig::new(name, size, true);
        for want in [MY_KEEPS, "Fray test host", "just now", "Join", "Forget"] {
            assert!(has(&rig.seen, want), "{name}: no {want:?} in the Join Game dialog: {:?}", rig.seen);
        }
        // The dialog's own parts are still there.
        assert!(has(&rig.seen, "Connect") && has(&rig.seen, "Cancel"), "{name}");
    }
}

#[test]
fn the_my_keeps_block_is_absent_when_there_are_no_keeps() {
    for (name, size) in [("desktop", DESKTOP), ("phone", PHONE)] {
        let rig = Rig::new(name, size, false);
        assert!(!has(&rig.seen, MY_KEEPS), "{name}: an empty list drew its heading");
        assert!(!has(&rig.seen, "Forget"), "{name}");
        assert!(has(&rig.seen, "Connect"), "{name}: the dialog itself is drawn");
    }
}

#[test]
fn join_in_the_dialog_hands_back_the_keeps_invite_with_no_confirm() {
    let mut rig = Rig::new("join", DESKTOP, true);
    assert!(matches!(rig.join, MenuAction::None));
    rig.tap("Join");
    match &rig.join {
        MenuAction::JoinKeep(invite) => assert_eq!((invite.host_key, invite.secret), (HOST, SECRET)),
        _ => panic!("a Join in the dialog did not give a dial of the saved Keep"),
    }
}

#[test]
fn forget_in_the_dialog_takes_two_taps_and_says_what_it_does() {
    let mut rig = Rig::new("forget", DESKTOP, true);
    rig.tap("Forget");
    assert_eq!(rig.keeps.len(), 1, "one tap only arms it");
    assert!(has(&rig.seen, "Forget for sure"), "the button asks again: {:?}", rig.seen);
    assert!(has(&rig.seen, FORGET_CONFIRM), "the sentence is on the screen: {:?}", rig.seen);
    assert!(matches!(rig.join, MenuAction::None), "Forget never dials");
    assert_eq!(crate::keeps::load_list(&rig.file.0).len(), 1, "the file still has the Keep");

    rig.tap("Forget for sure");
    assert!(rig.keeps.is_empty(), "the second tap forgets it");
    assert!(crate::keeps::load_list(&rig.file.0).is_empty(), "and the file");
    assert!(!has(&rig.seen, MY_KEEPS), "the block goes with the last Keep");
    assert!(matches!(rig.join, MenuAction::None));
}

#[test]
fn the_forget_sentence_is_inside_the_dialog_on_a_phone() {
    // The sentence under the buttons must not sit below the card's edge: the card scrolls it into view.
    let mut rig = Rig::new("forget-phone", PHONE, true);
    rig.tap("Forget");
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    let (_, at) = rig.seen.iter().find(|(t, _)| t == FORGET_CONFIRM).expect("the sentence is drawn");
    assert!(at.y > 0.0 && at.y < PHONE.y, "the sentence is off the screen at y={}", at.y);
}

#[test]
fn a_join_from_the_dialog_closes_it_and_dials_through_the_lobby() {
    // The real lobby arm: a Join in the open dialog closes it and returns the action.
    let _root = crate::save::WorldsRootGuard::new("join-dialog-keeps");
    let file = crate::keeps::path();
    let _ = std::fs::remove_file(&file);
    let invite = Invite { host_key: HOST, secret: SECRET };
    assert!(crate::keeps::save_invite_join(&file, &invite, Some("Fray test host"), crate::keeps::unix_now()));
    let mut state = MenuState::new();
    state.keeps = crate::keeps::load_list(&file);
    state.dialog = MenuDialog::JoinDirect { address: String::new() };

    let ctx = egui::Context::default();
    let mut frame = |state: &mut MenuState, events: Vec<egui::Event>| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, DESKTOP)),
            events,
            ..Default::default()
        };
        let mut action = MenuAction::None;
        let output = ctx.run(input, |ctx| action = draw_main_menu(ctx, state));
        (action, texts(&output))
    };
    let (_, _) = frame(&mut state, vec![]);
    let (_, seen) = frame(&mut state, vec![]);
    let at = seen.iter().find(|(t, _)| t == "Join").expect("a Join row in the open dialog").1;
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut state, vec![egui::Event::PointerMoved(at)]);
    frame(&mut state, vec![button(true)]);
    let (action, _) = frame(&mut state, vec![button(false)]);
    let _ = std::fs::remove_file(&file);
    match action {
        MenuAction::JoinKeep(got) => assert_eq!((got.host_key, got.secret), (HOST, SECRET)),
        _ => panic!("the lobby did not return a dial of the saved Keep"),
    }
    assert!(matches!(state.dialog, MenuDialog::None), "the dialog closed with no confirm");
    assert!(state.lan.is_none(), "and the LAN beacon port was freed");
}

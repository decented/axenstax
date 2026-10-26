//! My Keeps under the text box of the Join Game dialog, drawn headless (egui is pure CPU, so
//! no GPU is needed): the saved Keeps are listed at every screen width, the block is absent
//! when there are none, Join hands back the Keep's invite with no confirm and closes the
//! dialog, and Forget takes two taps. Click positions come from the text each frame drew.
//! On a compact (phone) card My Keeps is drawn first with the newest three rows and a More
//! Keeps / Fewer Keeps button, and no scroll area of its own: a swipe anywhere on the card
//! scrolls the card, and the code field is reachable however many Keeps there are.
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
pub(super) fn texts(output: &egui::FullOutput) -> Vec<(String, egui::Pos2)> {
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

/// Where a text was drawn: its rectangle and the clip rectangle it was drawn under (the
/// card's visible area, or that of whichever scroll area holds it).
pub(super) fn placed(output: &egui::FullOutput, label: &str) -> Option<(egui::Rect, egui::Rect)> {
    fn walk(shape: &egui::epaint::Shape, clip: egui::Rect, label: &str, out: &mut Option<(egui::Rect, egui::Rect)>) {
        match shape {
            egui::epaint::Shape::Text(t) if t.galley.text() == label && out.is_none() => {
                *out = Some((t.galley.rect.translate(t.pos.to_vec2()), clip));
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, label, out)),
            _ => {}
        }
    }
    let mut out = None;
    output.shapes.iter().for_each(|clipped| walk(&clipped.shape, clipped.clip_rect, label, &mut out));
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
    expanded: bool,
    join: MenuAction,
    file: TempFile,
    seen: Vec<(String, egui::Pos2)>,
    rooms: Vec<decented_net::LanRoom>,
    output: egui::FullOutput,
}

impl Rig {
    fn new(tag: &str, size: egui::Vec2, with_keep: bool) -> Rig {
        let mut rig = Rig::with(tag, size, 0, 0, None);
        if with_keep {
            rig.keeps = saved(&rig.file);
            rig.frame(vec![]);
            rig.frame(vec![]);
        }
        rig
    }

    /// `keeps` saved Keeps (the last saved is the newest), `rooms` heard LAN rooms, and the
    /// compact card forced on or off (`None`: as the screen size decides).
    fn with(tag: &str, size: egui::Vec2, keeps: usize, rooms: usize, compact: Option<bool>) -> Rig {
        let file = TempFile::new(tag);
        let now = crate::keeps::unix_now();
        for i in 0..keeps {
            let invite = Invite { host_key: [i as u8 + 1; 32], secret: SECRET };
            let name = format!("Fray test host {}", i + 1);
            assert!(crate::keeps::save_invite_join(&file.0, &invite, Some(name.as_str()), now - (keeps - i) as u64));
        }
        let keeps = crate::keeps::load_list(&file.0);
        let rooms = (0..rooms)
            .map(|i| decented_net::LanRoom {
                name: format!("LAN world {}", i + 1),
                code: format!("KMTRW{i}"),
                player_count: 1,
                max_players: 5,
                addr: std::net::SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, i as u8 + 1)), 40_000),
                token: "00".repeat(16),
            })
            .collect();
        let ctx = egui::Context::default();
        if let Some(on) = compact {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("test-force-compact-dialog"), on));
        }
        let mut rig = Rig { ctx, size, keeps, armed: None, expanded: false, join: MenuAction::None, file, seen: Vec::new(), rooms, output: Default::default() };
        // egui lays a widget out one frame before it can be hit, and a card that shrinks to its
        // content (centred on the screen) takes a few frames to settle: draw several first.
        for _ in 0..6 {
            rig.frame(vec![]);
        }
        rig
    }

    /// Is `label` drawn whole, inside the visible area of the scroll area holding it, which in
    /// turn lies inside the screen? That is: on the screen with no scrolling.
    fn on_screen(&self, label: &str) -> bool {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, self.size);
        match placed(&self.output, label) {
            Some((text, clip)) => clip.contains_rect(text) && screen.contains_rect(clip) && screen.contains_rect(text),
            None => false,
        }
    }

    fn y(&self, label: &str) -> f32 {
        placed(&self.output, label).unwrap_or_else(|| panic!("no {label:?} on screen: {:?}", self.seen)).0.center().y
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let lan = crate::lan_ui::LanGames { rooms: self.rooms.clone(), listening: true };
        let mut picked = None;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            events,
            ..Default::default()
        };
        let (keeps, armed, expanded, join, file) = (&mut self.keeps, &mut self.armed, &mut self.expanded, &mut self.join, &self.file.0);
        let output = self.ctx.run(input, |ctx| {
            let _ = draw_join_dialog(ctx, "", &lan, &mut picked, JoinKeepsPane { keeps, forget_armed: armed, expanded, file, join });
        });
        self.seen = texts(&output);
        self.output = output;
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

// ---- The compact card: My Keeps first, the newest three rows, More Keeps for the rest ----

/// A phone held upright, standard density (a Pixel 8 is 412 x 915 points).
const PORTRAIT: egui::Vec2 = egui::vec2(412.0, 915.0);

#[test]
fn a_compact_card_draws_my_keeps_above_the_lan_rooms_and_the_code_field() {
    for (name, size, compact) in [("portrait", PORTRAIT, Some(true)), ("landscape", PHONE, None), ("short", egui::vec2(412.0, 600.0), None)] {
        let rig = Rig::with(name, size, 1, 2, compact);
        let keeps = rig.y(MY_KEEPS);
        let rooms = rig.y(crate::lan_ui::GAMES_HEADING);
        let code = rig.y(crate::lan_ui::CODE_HEADING);
        assert!(keeps < rooms && rooms < code, "{name}: My Keeps {keeps}, LAN rooms {rooms}, code field {code}");
    }
}

#[test]
fn a_tall_window_keeps_my_keeps_under_the_text_box() {
    // Not compact: the layout the desktop has always had.
    for (name, size, compact) in [("desktop", DESKTOP, None), ("portrait-tall", PORTRAIT, Some(false))] {
        let rig = Rig::with(name, size, 1, 2, compact);
        let (rooms, code, keeps) = (rig.y(crate::lan_ui::GAMES_HEADING), rig.y(crate::lan_ui::CODE_HEADING), rig.y(MY_KEEPS));
        assert!(rooms < code && code < keeps, "{name}: LAN rooms {rooms}, code field {code}, My Keeps {keeps}");
    }
}

#[test]
fn a_saved_keeps_join_is_on_screen_with_no_scrolling_on_a_phone() {
    // One saved Keep and two heard LAN rooms, upright phone, compact card.
    let rig = Rig::with("first-row", PORTRAIT, 1, 2, Some(true));
    for want in [MY_KEEPS, "Fray test host 1", "Join", "Forget"] {
        assert!(rig.on_screen(want), "{want:?} is not inside the card's visible area: {:?}", rig.seen);
    }
    // And the LAN rows and the code field with it.
    for want in ["LAN world 1  \u{b7}  1 of 5 players", crate::lan_ui::CODE_HEADING, "Connect", "Cancel"] {
        assert!(rig.on_screen(want), "{want:?}: {:?}", rig.seen);
    }
}

#[test]
fn up_to_three_saved_keeps_show_every_row_and_no_button() {
    for n in [1usize, 3] {
        let rig = Rig::with("few", PORTRAIT, n, 2, Some(true));
        for i in 1..=n {
            let name = format!("Fray test host {i}");
            assert!(rig.on_screen(&name), "{n} Keeps: {name:?} missing: {:?}", rig.seen);
        }
        let joins = rig.seen.iter().filter(|(t, _)| t == "Join").count();
        assert_eq!(joins, n, "{n} Keeps: {:?}", rig.seen);
        assert!(!rig.seen.iter().any(|(t, _)| t.starts_with("More Keeps") || t == "Fewer Keeps"), "{n} Keeps drew a button: {:?}", rig.seen);
    }
}

#[test]
fn twenty_saved_keeps_show_three_rows_a_more_button_and_the_code_field() {
    let rig = Rig::with("twenty", PORTRAIT, 20, 2, Some(true));
    // Newest first: the newest Keep (the last saved) is the first row, and it is on screen.
    assert!(rig.on_screen("Fray test host 20"), "{:?}", rig.seen);
    assert!(rig.y("Fray test host 20") < rig.y(crate::lan_ui::CODE_HEADING));
    for want in ["Fray test host 20", "Fray test host 19", "Fray test host 18"] {
        assert!(rig.on_screen(want), "{want:?}: {:?}", rig.seen);
    }
    assert!(!has(&rig.seen, "Fray test host 17"), "a fourth row is drawn: {:?}", rig.seen);
    assert_eq!(rig.seen.iter().filter(|(t, _)| t == "Join").count(), 3);
    // The button counts the hidden Keeps, sits under the rows and above the code field.
    assert!(rig.on_screen("More Keeps (17)"), "{:?}", rig.seen);
    assert!(rig.y("Fray test host 18") < rig.y("More Keeps (17)") && rig.y("More Keeps (17)") < rig.y(crate::lan_ui::CODE_HEADING));
    // The code field and the rest are on screen with no scrolling at all.
    for want in [MY_KEEPS, "LAN world 1  \u{b7}  1 of 5 players", "LAN world 2  \u{b7}  1 of 5 players", crate::lan_ui::CODE_HEADING, crate::lan_ui::CODE_HELP, "Connect", "Cancel"] {
        assert!(rig.on_screen(want), "{want:?} is off the visible area with 20 Keeps: {:?}", rig.seen);
    }
}

#[test]
fn the_compact_my_keeps_rows_have_no_scroll_area_of_their_own() {
    // A nested scroll area clips its rows to its own rectangle and catches a swipe that starts
    // on them. With none, every row, the button and the code field share the card's clip.
    for expanded in [false, true] {
        let mut rig = Rig::with("no-nested", egui::vec2(412.0, 3000.0), 20, 2, Some(true));
        if expanded {
            rig.tap("More Keeps (17)");
        }
        let card = placed(&rig.output, crate::lan_ui::CODE_HEADING).expect("the code heading").1;
        let must_share = [MY_KEEPS, "Fray test host 20", "Fray test host 18", "Join", "Forget", if expanded { "Fewer Keeps" } else { "More Keeps (17)" }];
        for label in must_share {
            let clip = placed(&rig.output, label).unwrap_or_else(|| panic!("{label:?} not drawn: {:?}", rig.seen)).1;
            assert_eq!(clip, card, "expanded={expanded}: {label:?} is clipped by a scroll area of its own");
        }
    }
}

#[test]
fn more_keeps_shows_all_twenty_rows_and_fewer_keeps_folds_them_back() {
    // A screen tall enough to hold every row, so each one is drawn.
    let mut rig = Rig::with("expand", egui::vec2(412.0, 3000.0), 20, 2, Some(true));
    assert_eq!(rig.seen.iter().filter(|(t, _)| t == "Join").count(), 3);
    rig.tap("More Keeps (17)");
    assert_eq!(rig.seen.iter().filter(|(t, _)| t == "Join").count(), 20, "{:?}", rig.seen);
    for i in 1..=20 {
        let name = format!("Fray test host {i}");
        assert!(has(&rig.seen, &name), "{name:?} missing after More Keeps");
    }
    assert!(has(&rig.seen, "Fewer Keeps") && !rig.seen.iter().any(|(t, _)| t.starts_with("More Keeps")), "{:?}", rig.seen);
    assert!(rig.expanded, "the state is kept for as long as the dialog is open");
    // It stays expanded across frames, and folds back on the second tap.
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.seen.iter().filter(|(t, _)| t == "Join").count(), 20);
    rig.tap("Fewer Keeps");
    assert_eq!(rig.seen.iter().filter(|(t, _)| t == "Join").count(), 3, "{:?}", rig.seen);
    assert!(has(&rig.seen, "More Keeps (17)"));
}

#[test]
fn an_expanded_list_scrolls_with_the_card_down_to_the_code_field() {
    // On a phone, twenty rows do not fit: the card scrolls as a whole, so a wheel/swipe over a
    // row moves the card and the code field comes into view.
    let mut rig = Rig::with("expand-scroll", PORTRAIT, 20, 2, Some(true));
    rig.tap("More Keeps (17)");
    assert!(!rig.on_screen("Connect"), "twenty rows should push Connect off a phone: {:?}", rig.seen);
    let over_a_row = rig.seen.iter().find(|(t, _)| t == "Fray test host 20").unwrap().1;
    rig.frame(vec![egui::Event::PointerMoved(over_a_row)]);
    for _ in 0..8 {
        rig.frame(vec![egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, -400.0), phase: egui::TouchPhase::Move, modifiers: Default::default() }]);
    }
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    assert!(rig.on_screen("Connect"), "swiping over the rows did not reach the code field: {:?}", rig.seen);
    assert!(rig.on_screen(crate::lan_ui::CODE_HEADING), "{:?}", rig.seen);
}

#[test]
fn twenty_saved_keeps_leave_the_first_rows_join_on_a_landscape_phone() {
    // 914 x 411: the card scrolls as a whole, so the code field may need a swipe, but the
    // newest Keep's Join no longer does.
    let rig = Rig::with("twenty-landscape", PHONE, 20, 2, None);
    assert!(rig.on_screen("Fray test host 20"), "{:?}", rig.seen);
    assert!(rig.on_screen("Join"), "{:?}", rig.seen);
}

#[test]
fn forget_and_join_work_in_the_my_keeps_rows_on_a_compact_card() {
    let mut rig = Rig::with("compact-actions", PORTRAIT, 2, 1, Some(true));
    rig.tap("Forget");
    assert!(has(&rig.seen, "Forget for sure"));
    assert!(rig.on_screen(FORGET_CONFIRM), "the sentence is in view: {:?}", rig.seen);
    rig.tap("Forget for sure");
    assert_eq!(rig.keeps.len(), 1, "one Keep forgotten");
    // The card re-centres on the shorter list: let it settle before tapping.
    for _ in 0..4 {
        rig.frame(vec![]);
    }
    rig.tap("Join");
    assert!(matches!(rig.join, MenuAction::JoinKeep(_)));
}

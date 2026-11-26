//! Key bindings — the ONE table of what every key and mouse button does.
//!
//! Every keyboard / mouse reader goes through [`Bindings`]: `InputState`
//! (`input.rs`) turns a press into the flags `game_loop` reads, the chat keys in
//! `lib.rs` ask it which action a key is, and the held-key reads (movement, zoom,
//! free-look, the Director's fly keys) ask whether an action's binding is down.
//! The controls card (`controls.rs`) and the in-game key hints read their labels
//! from the same table, so what the game says is what the game does.
//!
//! Model (Spec 05 §1.9):
//! - An [`Action`] has a stable string id ([`Action::id`]). Persistence uses the
//!   id, never the enum's position, so reordering the enum loses nothing.
//! - A [`Binding`] is a physical key (`KeyCode`, the key's POSITION: on AZERTY
//!   the key the player pressed is the key they get) or one of five mouse buttons.
//! - Each action belongs to one [`Context`]. A binding is unique within its
//!   context; the same key may serve different contexts (E opens the inventory
//!   and turns a plan ghost clockwise). `InputState` sets the flag of EVERY
//!   context's action for a press and `game_loop` picks by context, as it always has.
//! - Esc and F1 to F12 are fixed: they keep a way out of any screen and the
//!   debug / camera / Director keys, are listed as fixed, and are never offered
//!   as a binding. The scroll wheel always cycles the hotbar.
//! - Only overrides are stored (`GraphicsSettings::key_bindings`, action id →
//!   binding name), so a player who never opens Settings → Controls keeps the
//!   defaults below, which reproduce the pre-remapping controls exactly
//!   (`tests::defaults_reproduce_the_pre_remapping_controls`).

use std::collections::BTreeMap;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// Where an action applies. A binding is unique within one context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Context {
    /// Normal play.
    Gameplay,
    /// A plan's ghost is being placed (`ghost_state`).
    BuildingPlan,
    /// The Workshop: the block painter and the avatar (skin) painter.
    Workshop,
    /// The cinematic Director camera.
    Director,
}

/// Which non-gameplay contexts are live this frame, told to `InputState` by the game
/// loop. A press that matches a live context's action is that context's: the Gameplay
/// action bound to the same key does not also fire (see [`Bindings::actions_firing`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActiveContexts {
    /// A plan's ghost is up (`ghost_state`).
    pub building_plan: bool,
    /// The Workshop: its world, or an avatar paint session.
    pub workshop: bool,
    /// An avatar (skin) paint session is open: the painter-only actions are live.
    pub avatar_paint: bool,
    /// The Director camera is on.
    pub director: bool,
}

impl ActiveContexts {
    /// Every context live (tests, the default-overlap parity check).
    #[cfg(test)]
    pub const ALL_LIVE: ActiveContexts =
        ActiveContexts { building_plan: true, workshop: true, avatar_paint: true, director: true };

    /// Is this non-gameplay action live now? Gameplay actions are never "owned".
    pub fn owns(self, action: Action) -> bool {
        use Action::*;
        match action.context() {
            Context::Gameplay => false,
            Context::BuildingPlan => self.building_plan,
            Context::Director => self.director,
            Context::Workshop => match action {
                // Only an avatar paint session reads these.
                PaintUndo | PaintRedo | PaintPicker | PaintPanel | PaintLimbs | PaintTool | PaintFill
                | PaintGrid | PaintLine => self.avatar_paint,
                _ => self.workshop,
            },
        }
    }
}

impl Context {
    pub const ALL: [Context; 4] = [
        Context::Gameplay,
        Context::BuildingPlan,
        Context::Workshop,
        Context::Director,
    ];

    /// The heading the Settings list groups this context's actions under.
    pub fn title(self) -> &'static str {
        match self {
            Context::Gameplay => "Playing",
            Context::BuildingPlan => "Building a plan",
            Context::Workshop => "Workshop painter",
            Context::Director => "Director camera",
        }
    }
}

/// Every keyboard / mouse action. Add a new one to [`Action::ALL`] and give it
/// a row in [`ACTION_TABLE`]; the tests check both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    // ── Gameplay ──
    Forward,
    Back,
    Left,
    Right,
    Jump,
    Sneak,
    Sprint,
    Attack,
    Use,
    PickBlock,
    Drop,
    SwapHands,
    Inventory,
    Hotbar1,
    Hotbar2,
    Hotbar3,
    Hotbar4,
    Hotbar5,
    Hotbar6,
    Hotbar7,
    Hotbar8,
    Hotbar9,
    Zoom,
    FreeLook,
    Explorer,
    Help,
    Map,
    Challenges,
    Satoshi,
    Chat,
    Command,
    RigStudio,
    // Fixed (Gameplay).
    Pause,
    DebugOverlay,
    ReplayRecord,
    CameraView,
    SpawnOverlay,
    Fullscreen,
    // ── Building a plan ──
    PlanRotateCw,
    PlanRotateCcw,
    // ── Workshop painter ──
    WorkshopPickColour,
    WorkshopMirror,
    WorkshopPin,
    WorkshopWardrobe,
    WorkshopMode,
    PaintUndo,
    PaintRedo,
    PaintPicker,
    PaintPanel,
    PaintLimbs,
    PaintTool,
    PaintFill,
    PaintGrid,
    PaintLine,
    // ── Director ──
    DirectorKeyframe,
    DirectorClearPath,
    // Fixed (Director).
    DirectorToggle,
    DirectorHud,
    DirectorMode,
    DirectorTarget,
    DirectorPlay,
}

/// How many actions there are.
pub const ACTION_COUNT: usize = Action::ALL.len();

/// One action's fixed facts: (action, id, name, context, default binding, fixed).
type ActionRow = (Action, &'static str, &'static str, Context, Binding, bool);

const fn key(k: KeyCode) -> Binding {
    Binding::Key(k)
}

const fn mouse(b: MouseButton) -> Binding {
    Binding::Mouse(b)
}

/// The table. Its order is the Settings list's order (and `Action::ALL`'s).
/// Each default is the key the code read before remapping existed; the parity
/// test pins them against those literals.
const ACTION_TABLE: [ActionRow; 61] = {
    use Action::*;
    use Context::*;
    [
        (Forward, "forward", "Walk forward", Gameplay, key(KeyCode::KeyW), false),
        (Back, "back", "Walk back", Gameplay, key(KeyCode::KeyS), false),
        (Left, "left", "Walk left", Gameplay, key(KeyCode::KeyA), false),
        (Right, "right", "Walk right", Gameplay, key(KeyCode::KeyD), false),
        (Jump, "jump", "Jump (double-tap: fly)", Gameplay, key(KeyCode::Space), false),
        (Sneak, "sneak", "Sneak", Gameplay, key(KeyCode::ShiftLeft), false),
        (Sprint, "sprint", "Sprint", Gameplay, key(KeyCode::ControlLeft), false),
        (Attack, "attack", "Attack / break", Gameplay, mouse(MouseButton::Left), false),
        (Use, "use", "Use / place", Gameplay, mouse(MouseButton::Right), false),
        (PickBlock, "pick_block", "Pick block", Gameplay, mouse(MouseButton::Middle), false),
        (Drop, "drop", "Drop", Gameplay, key(KeyCode::KeyQ), false),
        (SwapHands, "swap_hands", "Swap hands", Gameplay, key(KeyCode::KeyF), false),
        (Inventory, "inventory", "Inventory", Gameplay, key(KeyCode::KeyE), false),
        (Hotbar1, "hotbar_1", "Hotbar 1", Gameplay, key(KeyCode::Digit1), false),
        (Hotbar2, "hotbar_2", "Hotbar 2", Gameplay, key(KeyCode::Digit2), false),
        (Hotbar3, "hotbar_3", "Hotbar 3", Gameplay, key(KeyCode::Digit3), false),
        (Hotbar4, "hotbar_4", "Hotbar 4", Gameplay, key(KeyCode::Digit4), false),
        (Hotbar5, "hotbar_5", "Hotbar 5", Gameplay, key(KeyCode::Digit5), false),
        (Hotbar6, "hotbar_6", "Hotbar 6", Gameplay, key(KeyCode::Digit6), false),
        (Hotbar7, "hotbar_7", "Hotbar 7", Gameplay, key(KeyCode::Digit7), false),
        (Hotbar8, "hotbar_8", "Hotbar 8", Gameplay, key(KeyCode::Digit8), false),
        (Hotbar9, "hotbar_9", "Hotbar 9", Gameplay, key(KeyCode::Digit9), false),
        // pt1124 (0.2.43) — zooms only with a spyglass in a hand (`spyglass.rs`); the
        // id "zoom" stays so saved bindings still load.
        (Zoom, "zoom", "Zoom (hold, needs a spyglass)", Gameplay, key(KeyCode::KeyC), false),
        (FreeLook, "free_look", "Free-look (hold)", Gameplay, key(KeyCode::AltLeft), false),
        (Explorer, "explorer", "Search items", Gameplay, key(KeyCode::KeyB), false),
        (Help, "help", "Help", Gameplay, key(KeyCode::KeyH), false),
        (Map, "map", "Map", Gameplay, key(KeyCode::KeyM), false),
        (Challenges, "challenges", "Challenges", Gameplay, key(KeyCode::KeyJ), false),
        (Satoshi, "satoshi", "Call Satoshi", Gameplay, key(KeyCode::KeyN), false),
        (Chat, "chat", "Chat", Gameplay, key(KeyCode::KeyT), false),
        (Command, "command", "Command (chat with /)", Gameplay, key(KeyCode::Slash), false),
        (RigStudio, "rig_studio", "Rig Studio", Gameplay, key(KeyCode::KeyY), false),
        (Pause, "pause", "Pause / back", Gameplay, key(KeyCode::Escape), true),
        (DebugOverlay, "debug_overlay", "Debug overlay", Gameplay, key(KeyCode::F3), true),
        (ReplayRecord, "replay_record", "Record a replay", Gameplay, key(KeyCode::F4), true),
        (CameraView, "camera_view", "Camera view", Gameplay, key(KeyCode::F5), true),
        (SpawnOverlay, "spawn_overlay", "Mob-spawn overlay", Gameplay, key(KeyCode::F7), true),
        (Fullscreen, "fullscreen", "Fullscreen", Gameplay, key(KeyCode::F11), true),
        (PlanRotateCw, "plan_rotate_cw", "Turn clockwise", BuildingPlan, key(KeyCode::KeyE), false),
        (PlanRotateCcw, "plan_rotate_ccw", "Turn anticlockwise", BuildingPlan, key(KeyCode::KeyQ), false),
        (WorkshopPickColour, "workshop_pick_colour", "Pick colour", Workshop, key(KeyCode::KeyG), false),
        (WorkshopMirror, "workshop_mirror", "Mirror", Workshop, key(KeyCode::KeyM), false),
        (WorkshopPin, "workshop_pin", "Pin", Workshop, key(KeyCode::KeyP), false),
        (WorkshopWardrobe, "workshop_wardrobe", "Wardrobe", Workshop, key(KeyCode::KeyK), false),
        (WorkshopMode, "workshop_mode", "Paint / sculpt, clothes", Workshop, key(KeyCode::KeyV), false),
        (PaintUndo, "paint_undo", "Undo", Workshop, key(KeyCode::KeyZ), false),
        (PaintRedo, "paint_redo", "Redo", Workshop, key(KeyCode::KeyX), false),
        (PaintPicker, "paint_picker", "Colour picker", Workshop, key(KeyCode::KeyO), false),
        (PaintPanel, "paint_panel", "Paint panel", Workshop, key(KeyCode::Tab), false),
        (PaintLimbs, "paint_limbs", "Limbs apart", Workshop, key(KeyCode::KeyR), false),
        (PaintTool, "paint_tool", "Next brush", Workshop, key(KeyCode::KeyB), false),
        (PaintFill, "paint_fill", "Fill", Workshop, key(KeyCode::KeyF), false),
        (PaintGrid, "paint_grid", "Grid", Workshop, key(KeyCode::KeyL), false),
        (PaintLine, "paint_line", "Straight line (hold)", Workshop, key(KeyCode::ShiftLeft), false),
        (DirectorKeyframe, "director_keyframe", "Drop a keyframe", Director, key(KeyCode::Enter), false),
        (DirectorClearPath, "director_clear_path", "Clear the path", Director, key(KeyCode::Backspace), false),
        (DirectorToggle, "director_toggle", "Director on / off", Director, key(KeyCode::F6), true),
        (DirectorHud, "director_hud", "Hide the HUD", Director, key(KeyCode::F8), true),
        (DirectorMode, "director_mode", "Next mode", Director, key(KeyCode::F9), true),
        (DirectorTarget, "director_target", "Next target", Director, key(KeyCode::F10), true),
        (DirectorPlay, "director_play", "Play the path", Director, key(KeyCode::F12), true),
    ]
};

/// The table at a fixed address, for `&'static` row lookups at run time.
static ACTION_ROWS: [ActionRow; 61] = ACTION_TABLE;

impl Action {
    /// Every action, in the Settings list's order.
    pub const ALL: [Action; 61] = {
        let mut all = [Action::Forward; 61];
        let mut i = 0;
        while i < 61 {
            all[i] = ACTION_TABLE[i].0;
            i += 1;
        }
        all
    };

    fn row(self) -> &'static ActionRow {
        // The table is in enum order (checked by a test), so the discriminant
        // indexes it. In-memory only: nothing persists the position.
        &ACTION_ROWS[self as usize]
    }

    /// The stable id persisted in `settings.json` (`"drop"`, `"hotbar_1"`).
    pub fn id(self) -> &'static str {
        self.row().1
    }

    /// The action an id names, if any.
    pub fn from_id(id: &str) -> Option<Action> {
        ACTION_ROWS.iter().find(|r| r.1 == id).map(|r| r.0)
    }

    /// What the Settings list calls it ("Drop", "Hotbar 1").
    pub fn name(self) -> &'static str {
        self.row().2
    }

    pub fn context(self) -> Context {
        self.row().3
    }

    /// The shipped binding (today's controls).
    pub fn default_binding(self) -> Binding {
        self.row().4
    }

    /// Esc and the F-key actions: listed, never rebound.
    pub fn is_fixed(self) -> bool {
        self.row().5
    }

    /// The hotbar slot (0-8) a hotbar action selects.
    pub fn hotbar_slot(self) -> Option<usize> {
        use Action::*;
        [Hotbar1, Hotbar2, Hotbar3, Hotbar4, Hotbar5, Hotbar6, Hotbar7, Hotbar8, Hotbar9]
            .iter()
            .position(|a| *a == self)
    }
}

/// A physical key or a mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Binding {
    Key(KeyCode),
    Mouse(MouseButton),
}

/// Every key that can be named in `settings.json`: (key, stored name, label).
/// The stored name is winit's own name for the key (checked by a test). A key
/// that is not here cannot be bound (a capture refuses it).
const KEY_TABLE: &[(KeyCode, &str, &str)] = &[
    (KeyCode::KeyA, "KeyA", "A"),
    (KeyCode::KeyB, "KeyB", "B"),
    (KeyCode::KeyC, "KeyC", "C"),
    (KeyCode::KeyD, "KeyD", "D"),
    (KeyCode::KeyE, "KeyE", "E"),
    (KeyCode::KeyF, "KeyF", "F"),
    (KeyCode::KeyG, "KeyG", "G"),
    (KeyCode::KeyH, "KeyH", "H"),
    (KeyCode::KeyI, "KeyI", "I"),
    (KeyCode::KeyJ, "KeyJ", "J"),
    (KeyCode::KeyK, "KeyK", "K"),
    (KeyCode::KeyL, "KeyL", "L"),
    (KeyCode::KeyM, "KeyM", "M"),
    (KeyCode::KeyN, "KeyN", "N"),
    (KeyCode::KeyO, "KeyO", "O"),
    (KeyCode::KeyP, "KeyP", "P"),
    (KeyCode::KeyQ, "KeyQ", "Q"),
    (KeyCode::KeyR, "KeyR", "R"),
    (KeyCode::KeyS, "KeyS", "S"),
    (KeyCode::KeyT, "KeyT", "T"),
    (KeyCode::KeyU, "KeyU", "U"),
    (KeyCode::KeyV, "KeyV", "V"),
    (KeyCode::KeyW, "KeyW", "W"),
    (KeyCode::KeyX, "KeyX", "X"),
    (KeyCode::KeyY, "KeyY", "Y"),
    (KeyCode::KeyZ, "KeyZ", "Z"),
    (KeyCode::Digit0, "Digit0", "0"),
    (KeyCode::Digit1, "Digit1", "1"),
    (KeyCode::Digit2, "Digit2", "2"),
    (KeyCode::Digit3, "Digit3", "3"),
    (KeyCode::Digit4, "Digit4", "4"),
    (KeyCode::Digit5, "Digit5", "5"),
    (KeyCode::Digit6, "Digit6", "6"),
    (KeyCode::Digit7, "Digit7", "7"),
    (KeyCode::Digit8, "Digit8", "8"),
    (KeyCode::Digit9, "Digit9", "9"),
    (KeyCode::Backquote, "Backquote", "`"),
    (KeyCode::Minus, "Minus", "-"),
    (KeyCode::Equal, "Equal", "="),
    (KeyCode::BracketLeft, "BracketLeft", "["),
    (KeyCode::BracketRight, "BracketRight", "]"),
    (KeyCode::Backslash, "Backslash", "\\"),
    (KeyCode::IntlBackslash, "IntlBackslash", "\\ (left)"),
    (KeyCode::Semicolon, "Semicolon", ";"),
    (KeyCode::Quote, "Quote", "'"),
    (KeyCode::Comma, "Comma", ","),
    (KeyCode::Period, "Period", "."),
    (KeyCode::Slash, "Slash", "/"),
    (KeyCode::Space, "Space", "Space"),
    (KeyCode::Tab, "Tab", "Tab"),
    (KeyCode::Enter, "Enter", "Enter"),
    (KeyCode::Backspace, "Backspace", "Backspace"),
    (KeyCode::CapsLock, "CapsLock", "Caps Lock"),
    (KeyCode::ShiftLeft, "ShiftLeft", "Left Shift"),
    (KeyCode::ShiftRight, "ShiftRight", "Right Shift"),
    (KeyCode::ControlLeft, "ControlLeft", "Left Ctrl"),
    (KeyCode::ControlRight, "ControlRight", "Right Ctrl"),
    (KeyCode::AltLeft, "AltLeft", "Left Alt"),
    (KeyCode::AltRight, "AltRight", "Right Alt"),
    (KeyCode::ArrowUp, "ArrowUp", "Up"),
    (KeyCode::ArrowDown, "ArrowDown", "Down"),
    (KeyCode::ArrowLeft, "ArrowLeft", "Left"),
    (KeyCode::ArrowRight, "ArrowRight", "Right"),
    (KeyCode::Insert, "Insert", "Insert"),
    (KeyCode::Delete, "Delete", "Delete"),
    (KeyCode::Home, "Home", "Home"),
    (KeyCode::End, "End", "End"),
    (KeyCode::PageUp, "PageUp", "Page Up"),
    (KeyCode::PageDown, "PageDown", "Page Down"),
    (KeyCode::Numpad0, "Numpad0", "Numpad 0"),
    (KeyCode::Numpad1, "Numpad1", "Numpad 1"),
    (KeyCode::Numpad2, "Numpad2", "Numpad 2"),
    (KeyCode::Numpad3, "Numpad3", "Numpad 3"),
    (KeyCode::Numpad4, "Numpad4", "Numpad 4"),
    (KeyCode::Numpad5, "Numpad5", "Numpad 5"),
    (KeyCode::Numpad6, "Numpad6", "Numpad 6"),
    (KeyCode::Numpad7, "Numpad7", "Numpad 7"),
    (KeyCode::Numpad8, "Numpad8", "Numpad 8"),
    (KeyCode::Numpad9, "Numpad9", "Numpad 9"),
    (KeyCode::NumpadAdd, "NumpadAdd", "Numpad +"),
    (KeyCode::NumpadSubtract, "NumpadSubtract", "Numpad -"),
    (KeyCode::NumpadMultiply, "NumpadMultiply", "Numpad *"),
    (KeyCode::NumpadDivide, "NumpadDivide", "Numpad /"),
    (KeyCode::NumpadDecimal, "NumpadDecimal", "Numpad ."),
    (KeyCode::NumpadEnter, "NumpadEnter", "Numpad Enter"),
    // Fixed keys: named so a label can be shown, never stored as a binding.
    (KeyCode::Escape, "Escape", "Esc"),
    (KeyCode::F1, "F1", "F1"),
    (KeyCode::F2, "F2", "F2"),
    (KeyCode::F3, "F3", "F3"),
    (KeyCode::F4, "F4", "F4"),
    (KeyCode::F5, "F5", "F5"),
    (KeyCode::F6, "F6", "F6"),
    (KeyCode::F7, "F7", "F7"),
    (KeyCode::F8, "F8", "F8"),
    (KeyCode::F9, "F9", "F9"),
    (KeyCode::F10, "F10", "F10"),
    (KeyCode::F11, "F11", "F11"),
    (KeyCode::F12, "F12", "F12"),
];

/// The five mouse buttons a binding can name: (button, stored name, label).
const MOUSE_TABLE: [(MouseButton, &str, &str); 5] = [
    (MouseButton::Left, "MouseLeft", "Left click"),
    (MouseButton::Right, "MouseRight", "Right click"),
    (MouseButton::Middle, "MouseMiddle", "Middle click"),
    (MouseButton::Back, "MouseBack", "Mouse 4"),
    (MouseButton::Forward, "MouseForward", "Mouse 5"),
];

impl Binding {
    /// A mouse button a binding can name (`Other(n)` cannot).
    pub fn from_mouse(button: MouseButton) -> Option<Binding> {
        MOUSE_TABLE
            .iter()
            .any(|(b, _, _)| *b == button)
            .then_some(Binding::Mouse(button))
    }

    /// The name stored in `settings.json` (`"KeyQ"`, `"MouseBack"`), or `None`
    /// for a key this table does not know (it cannot be bound).
    pub fn name(self) -> Option<&'static str> {
        match self {
            Binding::Key(k) => KEY_TABLE.iter().find(|(c, _, _)| *c == k).map(|r| r.1),
            Binding::Mouse(m) => MOUSE_TABLE.iter().find(|(b, _, _)| *b == m).map(|r| r.1),
        }
    }

    /// The binding a stored name means.
    pub fn parse(name: &str) -> Option<Binding> {
        KEY_TABLE
            .iter()
            .find(|r| r.1 == name)
            .map(|r| Binding::Key(r.0))
            .or_else(|| MOUSE_TABLE.iter().find(|r| r.1 == name).map(|r| Binding::Mouse(r.0)))
    }

    /// The printed label shown to the player ("Q", "Left Shift", "Mouse 4"). The
    /// label of a letter is the US-layout name of the key's position.
    pub fn label(self) -> String {
        match self {
            Binding::Key(k) => KEY_TABLE
                .iter()
                .find(|(c, _, _)| *c == k)
                .map(|r| r.2.to_string())
                .unwrap_or_else(|| format!("{k:?}")),
            Binding::Mouse(m) => MOUSE_TABLE
                .iter()
                .find(|(b, _, _)| *b == m)
                .map(|r| r.2.to_string())
                .unwrap_or_else(|| format!("{m:?}")),
        }
    }

    /// The label a short in-game hint uses: the left-hand Shift, Ctrl and Alt
    /// read as plain "Shift", "Ctrl" and "Alt" (as the hints always have);
    /// everything else as [`Binding::label`].
    pub fn hint_label(self) -> String {
        match self {
            Binding::Key(KeyCode::ShiftLeft) => "Shift".to_string(),
            Binding::Key(KeyCode::ControlLeft) => "Ctrl".to_string(),
            Binding::Key(KeyCode::AltLeft) => "Alt".to_string(),
            other => other.label(),
        }
    }

    /// Tab: egui always takes it (it moves the keyboard focus through the menus), so a
    /// press never reaches the game and a binding on it could not fire. It cannot be
    /// chosen for an action. (Workshop's Paint panel still ships on it: see Spec 05 section 1.9.)
    pub fn is_menu_key(self) -> bool {
        matches!(self, Binding::Key(KeyCode::Tab))
    }

    /// Esc and F1 to F12: never a binding for a remappable action.
    pub fn is_fixed(self) -> bool {
        matches!(
            self,
            Binding::Key(
                KeyCode::Escape
                    | KeyCode::F1
                    | KeyCode::F2
                    | KeyCode::F3
                    | KeyCode::F4
                    | KeyCode::F5
                    | KeyCode::F6
                    | KeyCode::F7
                    | KeyCode::F8
                    | KeyCode::F9
                    | KeyCode::F10
                    | KeyCode::F11
                    | KeyCode::F12
            )
        )
    }
}

/// The outcome of asking for a new binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rebind {
    /// The action already had this binding.
    Unchanged,
    /// Bound; nothing else in the context used it.
    Bound,
    /// Bound; `other` used it in the same context and took the action's old
    /// binding `to` instead.
    Swapped { other: Action, to: Binding },
    /// The action is fixed (Esc, an F key).
    FixedAction,
    /// Esc or an F key was offered: refused.
    FixedKey,
    /// Tab was offered: refused (the menus use it).
    MenuKey,
    /// The binding is held by `other` in the same context and the swap would hand `other`
    /// Tab (the action's old binding), which no action can use: refused.
    WouldSwapOntoTab { other: Action },
    /// A key or button the table cannot name: refused.
    Unsupported,
}

impl Rebind {
    /// The short line the Settings list shows for this outcome, if any.
    pub fn message(self) -> Option<String> {
        match self {
            Rebind::Unchanged | Rebind::Bound => None,
            Rebind::Swapped { other, to } => Some(format!("{} moved to {}", other.name(), to.label())),
            Rebind::FixedAction => Some("That control can't be changed.".to_string()),
            Rebind::FixedKey => Some("Esc and F1 to F12 can't be used: they stay as they are.".to_string()),
            Rebind::MenuKey => Some("Tab can't be used: the menus use it.".to_string()),
            Rebind::WouldSwapOntoTab { other } => {
                Some(format!("{} would have to take Tab, which can't be used. Pick another key.", other.name()))
            }
            Rebind::Unsupported => Some("That key or button can't be used.".to_string()),
        }
    }
}

/// The live bindings: the defaults with the player's overrides applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bindings {
    by_action: [Binding; ACTION_COUNT],
}

impl Default for Bindings {
    fn default() -> Self {
        Self::defaults()
    }
}

impl Bindings {
    /// The shipped controls.
    pub fn defaults() -> Self {
        let mut by_action = [Binding::Key(KeyCode::KeyW); ACTION_COUNT];
        for a in Action::ALL {
            by_action[a as usize] = a.default_binding();
        }
        Bindings { by_action }
    }

    /// What the action is bound to now.
    pub fn binding_for(&self, action: Action) -> Binding {
        self.by_action[action as usize]
    }

    /// The action this binding triggers in `context`, if any.
    pub fn action_for(&self, context: Context, binding: Binding) -> Option<Action> {
        Action::ALL
            .into_iter()
            .find(|a| a.context() == context && self.by_action[*a as usize] == binding)
    }

    /// Every context's action for this binding (at most one per context).
    pub fn actions_for(&self, binding: Binding) -> impl Iterator<Item = Action> + '_ {
        Context::ALL.into_iter().filter_map(move |c| self.action_for(c, binding))
    }

    /// The actions a press of `binding` sets the flag of, given the live contexts.
    ///
    /// Every context's action bound to it fires (the game loop picks by its own state, as
    /// it always has), except that a Gameplay action does not fire while a live
    /// non-gameplay action holds the same binding: that context owns the press (the
    /// painter's Grid on Q must not also drop the dye). The shipped overlaps (E, Q, B, F, M:
    /// both actions on their default key) keep firing together, exactly as before
    /// remapping, because the game loop already tells those pairs apart.
    pub fn actions_firing(&self, binding: Binding, active: ActiveContexts) -> Vec<Action> {
        let all: Vec<Action> = self.actions_for(binding).collect();
        all.iter()
            .copied()
            .filter(|&a| {
                a.context() != Context::Gameplay
                    || !all.iter().any(|&o| {
                        o.context() != Context::Gameplay
                            && active.owns(o)
                            && !(a.default_binding() == binding && o.default_binding() == binding)
                    })
            })
            .collect()
    }

    /// The action's current binding as the player sees it ("Q", "Mouse 4").
    pub fn label(&self, action: Action) -> String {
        self.binding_for(action).label()
    }

    /// The action's binding as a short in-game hint names it ("Q", "Shift").
    pub fn hint(&self, action: Action) -> String {
        self.binding_for(action).hint_label()
    }

    /// True while the action has its shipped binding.
    pub fn is_default(&self, action: Action) -> bool {
        self.binding_for(action) == action.default_binding()
    }

    /// Bind `action` to `binding`. A fixed action or a fixed / unnamed key is
    /// refused; a binding another action of the same context holds is swapped:
    /// that action takes `action`'s old binding.
    pub fn rebind(&mut self, action: Action, binding: Binding) -> Rebind {
        if action.is_fixed() {
            return Rebind::FixedAction;
        }
        if binding.is_fixed() {
            return Rebind::FixedKey;
        }
        if binding.is_menu_key() {
            return Rebind::MenuKey;
        }
        if binding.name().is_none() {
            return Rebind::Unsupported;
        }
        let old = self.binding_for(action);
        if old == binding {
            return Rebind::Unchanged;
        }
        let clash = self.action_for(action.context(), binding);
        if let Some(other) = clash
            && old.is_menu_key()
        {
            return Rebind::WouldSwapOntoTab { other };
        }
        self.by_action[action as usize] = binding;
        match clash {
            Some(other) => {
                self.by_action[other as usize] = old;
                Rebind::Swapped { other, to: old }
            }
            None => Rebind::Bound,
        }
    }

    /// The player's changes only (action id → binding name), as stored in
    /// `GraphicsSettings::key_bindings`.
    pub fn overrides(&self) -> BTreeMap<String, String> {
        Action::ALL
            .into_iter()
            .filter(|a| !a.is_fixed() && !self.is_default(*a))
            .filter_map(|a| Some((a.id().to_string(), self.binding_for(a).name()?.to_string())))
            .collect()
    }

    /// Build the live bindings from stored overrides. Returns what was ignored,
    /// one line each, for the caller to log once: an unknown action id or
    /// binding name, a fixed action or key, and any action left sharing a
    /// binding with another in its context (those actions go back to default,
    /// repeated until no context has a clash; the defaults have none).
    pub fn from_overrides(map: &BTreeMap<String, String>) -> (Bindings, Vec<String>) {
        let mut b = Self::defaults();
        let mut ignored = Vec::new();
        for (id, name) in map {
            let Some(action) = Action::from_id(id) else {
                ignored.push(format!("unknown control '{id}'"));
                continue;
            };
            if action.is_fixed() {
                ignored.push(format!("'{id}' is fixed"));
                continue;
            }
            let Some(binding) = Binding::parse(name) else {
                ignored.push(format!("unknown key '{name}' for '{id}'"));
                continue;
            };
            if binding.is_fixed() {
                ignored.push(format!("'{name}' is a fixed key ('{id}')"));
                continue;
            }
            if binding.is_menu_key() {
                ignored.push(format!("'{name}' is a menu key ('{id}')"));
                continue;
            }
            b.by_action[action as usize] = binding;
        }
        // Clashes: every overridden action in a clashing group goes back to
        // default, the whole group at once. Reverting can make a new clash
        // with a default, so repeat; each pass removes at least one override
        // (the defaults have no clash), so this ends.
        loop {
            let clashing: Vec<Action> = Action::ALL
                .into_iter()
                .filter(|a| {
                    !b.is_default(*a)
                        && Action::ALL.into_iter().any(|o| {
                            o != *a && o.context() == a.context() && b.binding_for(o) == b.binding_for(*a)
                        })
                })
                .collect();
            if clashing.is_empty() {
                break;
            }
            for a in clashing {
                ignored.push(format!("'{}' shared {} with another control", a.id(), b.label(a)));
                b.by_action[a as usize] = a.default_binding();
            }
        }
        (b, ignored)
    }
}

/// How a key press reaches the game, decided in `lib.rs` after egui has seen
/// it. (A Settings capture is decided earlier, before egui: see
/// `keybindings_ui::KeyBindingsUi::capture_active`.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRoute {
    /// Gameplay input: bindings fire.
    Play,
    /// Chat is open or a text field has the keyboard: no binding fires; only a
    /// release goes through, so a key held before the focus moved can't stick.
    ReleaseOnly,
}

/// Where a key event goes once egui has seen it: `chat_open` is the chat
/// overlay, `text_focus` is "egui consumed it" (a focused text field).
pub fn route_key(chat_open: bool, text_focus: bool) -> KeyRoute {
    if chat_open || text_focus {
        KeyRoute::ReleaseOnly
    } else {
        KeyRoute::Play
    }
}

/// A workstation window (the furnace) closes on the Inventory binding as it opens the
/// inventory, even when egui has consumed the key because a slot button holds the keyboard
/// focus (Tab, the arrows). Only a focused text field keeps the key. `lib.rs` asks this
/// before dropping a consumed press.
pub fn workstation_close_passes(
    bindings: &Bindings,
    binding: Binding,
    workstation_open: bool,
    text_edit_focused: bool,
) -> bool {
    workstation_open
        && !text_edit_focused
        && bindings.action_for(Context::Gameplay, binding) == Some(Action::Inventory)
}

/// Which toggled panels are open now (the ones a Gameplay toggle action opens and closes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenPanels {
    /// The inventory / crafting window, or a furnace (Inventory closes it).
    pub inventory: bool,
    pub explorer: bool,
    pub map: bool,
    pub challenges: bool,
    pub rig_studio: bool,
    pub help: bool,
}

/// A mouse button bound to a panel's toggle, pressed while that panel is open, closes it.
/// A mouse press only reaches `InputState` while the cursor is captured for play, and an
/// open panel frees the cursor, so `lib.rs` asks this before the usual click handling. A
/// focused text field keeps the press.
///
/// `egui_took_press` is egui-winit's "consumed" for the press, which is true whenever the
/// pointer is over ANY egui window (the freed cursor always is, over the panel it just
/// opened), not only over a widget. The side buttons (Mouse 4 and 5) are read by no widget
/// (egui's `clicked()` is the primary button only and the engine uses no `clicked_by` for
/// them), so they close the panel wherever the pointer is. Left, Right and Middle may be a
/// widget's click, so egui keeps those.
pub fn mouse_toggle_closes_panel(
    bindings: &Bindings,
    binding: Binding,
    open: OpenPanels,
    text_edit_focused: bool,
    egui_took_press: bool,
) -> bool {
    let Binding::Mouse(button) = binding else {
        return false;
    };
    if text_edit_focused {
        return false;
    }
    if egui_took_press && !matches!(button, MouseButton::Back | MouseButton::Forward) {
        return false;
    }
    match bindings.action_for(Context::Gameplay, binding) {
        Some(Action::Inventory) => open.inventory,
        Some(Action::Explorer) => open.explorer,
        Some(Action::Map) => open.map,
        Some(Action::Challenges) => open.challenges,
        Some(Action::RigStudio) => open.rig_studio,
        Some(Action::Help) => open.help,
        _ => false,
    }
}

/// The chat line a press opens with, if it is bound to Chat (`""`) or
/// Command (`"/"`) during play. `lib.rs` asks this for keys and mouse buttons.
pub fn chat_prefix(bindings: &Bindings, binding: Binding) -> Option<&'static str> {
    match bindings.action_for(Context::Gameplay, binding)? {
        Action::Chat => Some(""),
        Action::Command => Some("/"),
        _ => None,
    }
}

/// One log line for what a stored overrides map had that was ignored.
pub fn log_ignored(ignored: &[String]) {
    if !ignored.is_empty() {
        log::warn!("Controls: ignored {} stored binding(s): {}", ignored.len(), ignored.join("; "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_in_enum_order_and_every_id_is_unique() {
        for (i, row) in ACTION_TABLE.iter().enumerate() {
            assert_eq!(row.0 as usize, i, "{:?} is out of order in ACTION_TABLE", row.0);
            assert_eq!(Action::ALL[i], row.0);
        }
        let mut ids = std::collections::HashSet::new();
        for a in Action::ALL {
            assert!(ids.insert(a.id()), "duplicate id {}", a.id());
            assert_eq!(Action::from_id(a.id()), Some(a));
            assert!(!a.name().is_empty());
        }
        assert_eq!(Action::from_id("no_such_control"), None);
    }

    /// The defaults reproduce the controls as they were before remapping, key
    /// for key: each literal below is the one `input.rs`, `lib.rs` and
    /// `game_loop.rs` matched on. A player who never opens Settings → Controls
    /// plays exactly as before.
    #[test]
    fn defaults_reproduce_the_pre_remapping_controls() {
        use Action::*;
        let k = Binding::Key;
        let m = Binding::Mouse;
        let today: [(Action, Binding); 61] = [
            (Forward, k(KeyCode::KeyW)),
            (Back, k(KeyCode::KeyS)),
            (Left, k(KeyCode::KeyA)),
            (Right, k(KeyCode::KeyD)),
            (Jump, k(KeyCode::Space)),
            (Sneak, k(KeyCode::ShiftLeft)),
            (Sprint, k(KeyCode::ControlLeft)),
            (Attack, m(MouseButton::Left)),
            (Use, m(MouseButton::Right)),
            (PickBlock, m(MouseButton::Middle)),
            (Drop, k(KeyCode::KeyQ)),
            (SwapHands, k(KeyCode::KeyF)),
            (Inventory, k(KeyCode::KeyE)),
            (Hotbar1, k(KeyCode::Digit1)),
            (Hotbar2, k(KeyCode::Digit2)),
            (Hotbar3, k(KeyCode::Digit3)),
            (Hotbar4, k(KeyCode::Digit4)),
            (Hotbar5, k(KeyCode::Digit5)),
            (Hotbar6, k(KeyCode::Digit6)),
            (Hotbar7, k(KeyCode::Digit7)),
            (Hotbar8, k(KeyCode::Digit8)),
            (Hotbar9, k(KeyCode::Digit9)),
            (Zoom, k(KeyCode::KeyC)),
            (FreeLook, k(KeyCode::AltLeft)),
            (Explorer, k(KeyCode::KeyB)),
            (Help, k(KeyCode::KeyH)),
            (Map, k(KeyCode::KeyM)),
            (Challenges, k(KeyCode::KeyJ)),
            (Satoshi, k(KeyCode::KeyN)),
            (Chat, k(KeyCode::KeyT)),
            (Command, k(KeyCode::Slash)),
            (RigStudio, k(KeyCode::KeyY)),
            (Pause, k(KeyCode::Escape)),
            (DebugOverlay, k(KeyCode::F3)),
            (ReplayRecord, k(KeyCode::F4)),
            (CameraView, k(KeyCode::F5)),
            (SpawnOverlay, k(KeyCode::F7)),
            (Fullscreen, k(KeyCode::F11)),
            (PlanRotateCw, k(KeyCode::KeyE)),
            (PlanRotateCcw, k(KeyCode::KeyQ)),
            (WorkshopPickColour, k(KeyCode::KeyG)),
            (WorkshopMirror, k(KeyCode::KeyM)),
            (WorkshopPin, k(KeyCode::KeyP)),
            (WorkshopWardrobe, k(KeyCode::KeyK)),
            (WorkshopMode, k(KeyCode::KeyV)),
            (PaintUndo, k(KeyCode::KeyZ)),
            (PaintRedo, k(KeyCode::KeyX)),
            (PaintPicker, k(KeyCode::KeyO)),
            (PaintPanel, k(KeyCode::Tab)),
            (PaintLimbs, k(KeyCode::KeyR)),
            (PaintTool, k(KeyCode::KeyB)),
            (PaintFill, k(KeyCode::KeyF)),
            (PaintGrid, k(KeyCode::KeyL)),
            (PaintLine, k(KeyCode::ShiftLeft)),
            (DirectorKeyframe, k(KeyCode::Enter)),
            (DirectorClearPath, k(KeyCode::Backspace)),
            (DirectorToggle, k(KeyCode::F6)),
            (DirectorHud, k(KeyCode::F8)),
            (DirectorMode, k(KeyCode::F9)),
            (DirectorTarget, k(KeyCode::F10)),
            (DirectorPlay, k(KeyCode::F12)),
        ];
        let b = Bindings::defaults();
        for (a, want) in today {
            assert_eq!(b.binding_for(a), want, "{} ({:?}) changed its default", a.id(), a);
            assert_eq!(a.default_binding(), want);
        }
        assert!(b.overrides().is_empty(), "the defaults store nothing");
        // Exactly the fixed keys are fixed.
        for a in Action::ALL {
            assert_eq!(a.is_fixed(), a.default_binding().is_fixed(), "{}", a.id());
        }
    }

    #[test]
    fn the_defaults_have_no_clash_inside_any_context() {
        let b = Bindings::defaults();
        for a in Action::ALL {
            for o in Action::ALL {
                if a != o && a.context() == o.context() {
                    assert_ne!(b.binding_for(a), b.binding_for(o), "{} and {} share a key", a.id(), o.id());
                }
            }
        }
        // E and Q serve two contexts, as they always have.
        let e = Binding::Key(KeyCode::KeyE);
        assert_eq!(b.actions_for(e).collect::<Vec<_>>(), vec![Action::Inventory, Action::PlanRotateCw]);
    }

    #[test]
    fn every_key_name_is_winits_own_and_round_trips() {
        for (code, name, label) in KEY_TABLE {
            assert_eq!(format!("{code:?}"), *name);
            assert!(!label.is_empty());
            assert_eq!(Binding::parse(name), Some(Binding::Key(*code)));
        }
        for (button, name, _) in MOUSE_TABLE {
            assert_eq!(Binding::parse(name), Some(Binding::Mouse(button)));
            assert_eq!(Binding::from_mouse(button), Some(Binding::Mouse(button)));
        }
        assert_eq!(Binding::from_mouse(MouseButton::Other(7)), None);
        assert_eq!(Binding::Key(KeyCode::KeyQ).label(), "Q");
        assert_eq!(Binding::Key(KeyCode::ShiftLeft).label(), "Left Shift");
        assert_eq!(Binding::Mouse(MouseButton::Back).label(), "Mouse 4");
    }

    #[test]
    fn overrides_round_trip_through_json() {
        let mut b = Bindings::defaults();
        assert_eq!(b.rebind(Action::Drop, Binding::Key(KeyCode::KeyG)), Rebind::Bound);
        assert_eq!(b.rebind(Action::Hotbar1, Binding::Mouse(MouseButton::Back)), Rebind::Bound);
        let map = b.overrides();
        assert_eq!(map.len(), 2, "only the changes are stored: {map:?}");
        assert_eq!(map.get("drop").map(String::as_str), Some("KeyG"));
        assert_eq!(map.get("hotbar_1").map(String::as_str), Some("MouseBack"));

        let gfx = crate::graphics_settings::GraphicsSettings { key_bindings: map, ..Default::default() };
        let json = serde_json::to_string(&gfx).unwrap();
        let back: crate::graphics_settings::GraphicsSettings = serde_json::from_str(&json).unwrap();
        let (loaded, ignored) = Bindings::from_overrides(&back.key_bindings);
        assert!(ignored.is_empty(), "{ignored:?}");
        assert_eq!(loaded, b);
    }

    #[test]
    fn unknown_ids_names_fixed_keys_and_clashes_are_ignored_on_load() {
        let mut map = BTreeMap::new();
        map.insert("no_such_control".to_string(), "KeyG".to_string());
        map.insert("drop".to_string(), "NotAKey".to_string());
        map.insert("pause".to_string(), "KeyP".to_string());
        map.insert("map".to_string(), "F5".to_string());
        // A bad file: two gameplay actions on one key.
        map.insert("help".to_string(), "KeyU".to_string());
        map.insert("satoshi".to_string(), "KeyU".to_string());
        // A clash that only appears once another override is dropped: zoom
        // takes Help's default H while Help moves off it — then Help reverts
        // to H and must take zoom back to default too.
        map.insert("zoom".to_string(), "KeyH".to_string());
        // A good one survives.
        map.insert("inventory".to_string(), "KeyI".to_string());
        let (b, ignored) = Bindings::from_overrides(&map);
        assert!(ignored.len() >= 6, "each problem is reported: {ignored:?}");
        let d = Bindings::defaults();
        for a in [Action::Drop, Action::Pause, Action::Map, Action::Help, Action::Satoshi, Action::Zoom] {
            assert_eq!(b.binding_for(a), d.binding_for(a), "{} fell back to default", a.id());
        }
        assert_eq!(b.binding_for(Action::Inventory), Binding::Key(KeyCode::KeyI));
        for a in Action::ALL {
            for o in Action::ALL {
                if a != o && a.context() == o.context() {
                    assert_ne!(b.binding_for(a), b.binding_for(o), "{} and {} clash after load", a.id(), o.id());
                }
            }
        }
    }

    #[test]
    fn a_binding_used_in_the_same_context_is_swapped() {
        let mut b = Bindings::defaults();
        // Drop takes Inventory's E: Inventory moves to Q, Drop's old key.
        let r = b.rebind(Action::Drop, Binding::Key(KeyCode::KeyE));
        assert_eq!(r, Rebind::Swapped { other: Action::Inventory, to: Binding::Key(KeyCode::KeyQ) });
        assert_eq!(b.binding_for(Action::Drop), Binding::Key(KeyCode::KeyE));
        assert_eq!(b.binding_for(Action::Inventory), Binding::Key(KeyCode::KeyQ));
        assert_eq!(r.message().as_deref(), Some("Inventory moved to Q"));
        // A key used only in ANOTHER context is no clash: Drop on G (the
        // Workshop's pick-colour key) leaves pick-colour alone.
        let mut b = Bindings::defaults();
        assert_eq!(b.rebind(Action::Drop, Binding::Key(KeyCode::KeyG)), Rebind::Bound);
        assert_eq!(b.binding_for(Action::WorkshopPickColour), Binding::Key(KeyCode::KeyG));
        // The brief's own example line.
        let mut b = Bindings::defaults();
        b.rebind(Action::Drop, Binding::Key(KeyCode::KeyG));
        let r = b.rebind(Action::Satoshi, Binding::Key(KeyCode::KeyG));
        assert_eq!(r.message().as_deref(), Some("Drop moved to N"));
    }

    #[test]
    fn every_default_overlap_keeps_firing_together_with_all_contexts_live() {
        // Parity with 0.2.40: wherever two actions of different contexts ship on one key,
        // a press fires both (the game loop picks), whatever is live.
        let b = Bindings::defaults();
        let mut overlaps = 0;
        for a in Action::ALL {
            let binding = a.default_binding();
            let all: Vec<Action> = b.actions_for(binding).collect();
            if all.len() > 1 {
                overlaps += 1;
                for active in [ActiveContexts::ALL_LIVE, ActiveContexts::default()] {
                    assert_eq!(b.actions_firing(binding, active), all, "{} on {}", a.id(), binding.label());
                }
            }
        }
        assert!(overlaps >= 5, "E, Q, B, F and M overlap by default");
    }

    #[test]
    fn a_live_context_owns_a_rebound_key_but_only_while_live() {
        let mut b = Bindings::defaults();
        b.rebind(Action::PaintGrid, Binding::Key(KeyCode::KeyQ));
        let q = Binding::Key(KeyCode::KeyQ);
        let live = ActiveContexts { workshop: true, avatar_paint: true, ..Default::default() };
        let fired = b.actions_firing(q, live);
        assert!(fired.contains(&Action::PaintGrid) && !fired.contains(&Action::Drop));
        // The ghost's own Q is not Gameplay, so it is not silenced by the painter.
        assert!(fired.contains(&Action::PlanRotateCcw));
        let fired = b.actions_firing(q, ActiveContexts { workshop: true, ..Default::default() });
        assert!(fired.contains(&Action::Drop), "a Workshop world without a paint session does not own Grid");
    }

    #[test]
    fn the_inventory_key_closes_a_furnace_unless_a_text_field_has_the_keyboard() {
        let b = Bindings::defaults();
        let e = Binding::Key(KeyCode::KeyE);
        assert!(workstation_close_passes(&b, e, true, false), "E, a slot button focused");
        assert!(!workstation_close_passes(&b, e, true, true), "a text field keeps the key");
        assert!(!workstation_close_passes(&b, e, false, false), "no workstation open");
        assert!(!workstation_close_passes(&b, Binding::Key(KeyCode::KeyQ), true, false), "another key");
        // Follows a rebound key.
        let mut b = Bindings::defaults();
        b.rebind(Action::Inventory, Binding::Key(KeyCode::KeyI));
        assert!(workstation_close_passes(&b, Binding::Key(KeyCode::KeyI), true, false));
        assert!(!workstation_close_passes(&b, e, true, false));
    }

    #[test]
    fn a_focused_slot_button_is_not_a_text_field_to_egui() {
        // The scenario behind the check: Tab onto a button makes egui want the keyboard
        // (so its `consumed` is true) while no text edit holds it.
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.button("slot").request_focus();
            });
        }
        assert!(ctx.egui_wants_keyboard_input(), "a focused button takes the keyboard from the game");
        assert!(!ctx.text_edit_focused(), "but it is not a text field");
    }

    #[test]
    fn a_mouse_button_bound_to_a_toggle_closes_its_open_panel() {
        let mut b = Bindings::defaults();
        b.rebind(Action::Inventory, Binding::Mouse(MouseButton::Forward));
        b.rebind(Action::Map, Binding::Mouse(MouseButton::Back));
        let fwd = Binding::Mouse(MouseButton::Forward);
        let back = Binding::Mouse(MouseButton::Back);
        let inv = OpenPanels { inventory: true, ..Default::default() };
        assert!(mouse_toggle_closes_panel(&b, fwd, inv, false, false), "Mouse 5 closes the open inventory");
        assert!(!mouse_toggle_closes_panel(&b, back, inv, false, false), "Map is not open");
        assert!(mouse_toggle_closes_panel(&b, back, OpenPanels { map: true, ..Default::default() }, false, false));
        assert!(!mouse_toggle_closes_panel(&b, fwd, inv, true, false), "a focused text field keeps it");
        assert!(!mouse_toggle_closes_panel(&b, fwd, OpenPanels::default(), false, false), "nothing open");
        // A key binding is not a mouse press; Attack on its click is no toggle.
        assert!(!mouse_toggle_closes_panel(&b, Binding::Key(KeyCode::KeyE), inv, false, false));
        assert!(!mouse_toggle_closes_panel(&b, Binding::Mouse(MouseButton::Left), inv, false, false));
        // The freed cursor sits over the panel, so egui "took" every press: the side
        // buttons (no widget reads them) still close it; the other three stay egui's.
        assert!(mouse_toggle_closes_panel(&b, fwd, inv, false, true), "Mouse 5 over the open window");
        let mut c = Bindings::defaults();
        c.rebind(Action::Inventory, Binding::Mouse(MouseButton::Middle));
        let mid = Binding::Mouse(MouseButton::Middle);
        assert!(mouse_toggle_closes_panel(&c, mid, inv, false, false));
        assert!(!mouse_toggle_closes_panel(&c, mid, inv, false, true), "a widget may take a middle click");
    }

    #[test]
    fn a_fixed_key_or_action_is_refused() {
        let mut b = Bindings::defaults();
        assert_eq!(b.rebind(Action::Drop, Binding::Key(KeyCode::Escape)), Rebind::FixedKey);
        assert_eq!(b.rebind(Action::Drop, Binding::Key(KeyCode::F5)), Rebind::FixedKey);
        assert_eq!(b.rebind(Action::Pause, Binding::Key(KeyCode::KeyP)), Rebind::FixedAction);
        assert_eq!(b.rebind(Action::Drop, Binding::Key(KeyCode::F13)), Rebind::Unsupported);
        assert_eq!(b, Bindings::defaults(), "nothing changed");
        assert!(Rebind::FixedKey.message().is_some());
    }

    #[test]
    fn tab_is_refused_as_a_binding_and_ignored_in_settings() {
        let mut b = Bindings::defaults();
        assert_eq!(b.rebind(Action::Hotbar1, Binding::Key(KeyCode::Tab)), Rebind::MenuKey);
        assert_eq!(b, Bindings::defaults(), "nothing changed");
        assert_eq!(Rebind::MenuKey.message().as_deref(), Some("Tab can't be used: the menus use it."));
        let mut map = BTreeMap::new();
        map.insert("hotbar_1".to_string(), "Tab".to_string());
        map.insert("inventory".to_string(), "KeyI".to_string());
        let (loaded, ignored) = Bindings::from_overrides(&map);
        assert_eq!(loaded.binding_for(Action::Hotbar1), Action::Hotbar1.default_binding());
        assert_eq!(loaded.binding_for(Action::Inventory), Binding::Key(KeyCode::KeyI), "the rest still loads");
        assert_eq!(ignored.len(), 1, "one line for it: {ignored:?}");
        assert!(ignored[0].contains("Tab"));
    }

    #[test]
    fn the_paint_panels_shipped_tab_never_passes_to_another_action_by_a_swap() {
        // Moving Paint panel onto a key another Workshop action holds would hand that
        // action Tab; it is refused instead, and nothing changes.
        let mut b = Bindings::defaults();
        let held = Action::ALL
            .into_iter()
            .find(|a| a.context() == Action::PaintPanel.context() && *a != Action::PaintPanel)
            .unwrap();
        let r = b.rebind(Action::PaintPanel, b.binding_for(held));
        assert_eq!(r, Rebind::WouldSwapOntoTab { other: held });
        assert_eq!(b, Bindings::defaults());
        // A free key moves it as ever.
        assert_eq!(b.rebind(Action::PaintPanel, Binding::Key(KeyCode::Backquote)), Rebind::Bound);
    }

    #[test]
    fn a_text_field_or_chat_stops_bindings_firing() {
        assert_eq!(route_key(false, false), KeyRoute::Play);
        assert_eq!(route_key(false, true), KeyRoute::ReleaseOnly, "a focused text field");
        assert_eq!(route_key(true, false), KeyRoute::ReleaseOnly, "chat open");
        // A real focused egui text field reports the keyboard as taken, which
        // is what `lib.rs` passes as `text_focus` (egui-winit's `consumed`).
        let ctx = egui::Context::default();
        let mut text = String::new();
        let field = egui::Id::new("kb-test-field");
        for _ in 0..3 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut text).id(field));
                r.request_focus();
            });
        }
        assert!(ctx.egui_wants_keyboard_input(), "the field holds the keyboard");
        assert_eq!(route_key(false, ctx.egui_wants_keyboard_input()), KeyRoute::ReleaseOnly);
    }

    #[test]
    fn chat_and_command_follow_their_bindings() {
        let mut b = Bindings::defaults();
        assert_eq!(chat_prefix(&b, Binding::Key(KeyCode::KeyT)), Some(""));
        assert_eq!(chat_prefix(&b, Binding::Key(KeyCode::Slash)), Some("/"));
        assert_eq!(chat_prefix(&b, Binding::Key(KeyCode::KeyE)), None);
        b.rebind(Action::Chat, Binding::Key(KeyCode::KeyU));
        b.rebind(Action::Command, Binding::Mouse(MouseButton::Forward));
        assert_eq!(chat_prefix(&b, Binding::Key(KeyCode::KeyU)), Some(""));
        assert_eq!(chat_prefix(&b, Binding::Key(KeyCode::KeyT)), None, "T no longer opens chat");
        assert_eq!(chat_prefix(&b, Binding::Mouse(MouseButton::Forward)), Some("/"));
    }

    #[test]
    fn hints_name_the_left_modifiers_plainly() {
        let b = Bindings::defaults();
        assert_eq!(b.hint(Action::Sneak), "Shift");
        assert_eq!(b.hint(Action::Sprint), "Ctrl");
        assert_eq!(b.hint(Action::FreeLook), "Alt");
        assert_eq!(b.hint(Action::Drop), "Q");
        assert_eq!(b.label(Action::Sneak), "Left Shift", "the Settings list stays exact");
    }
}

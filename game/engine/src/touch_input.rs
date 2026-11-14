//! Touch input for WASM — converts touch events to `PlayerIntent` and draws the
//! on-screen control overlay.
//!
//! ## Layout (Minecraft Bedrock / Education parity)
//!
//! The scheme mirrors what a player coming from Minecraft on a touchscreen
//! Chromebook/tablet already knows (the "Joystick + Split-Controls" scheme —
//! crosshair in the centre, dedicated action buttons):
//!
//! ```text
//!  [Pause][Chat]                                         [View]
//!                                                              [Zoom]
//!
//!                            (drag anywhere = look)
//!
//!                                                   [Place][Break]
//!     (  joystick  )                               [Sneak][Jump ]
//!                          [ hotbar 1..9 ] [Inv]
//! ```
//!
//! - **Bottom-left** — floating virtual joystick (move; push to the edge to
//!   sprint). A faint "home" ring shows where to plant the thumb before it's
//!   grabbed.
//! - **Bottom-right** — the 2×2 action cluster: Jump + Sneak on the lower row,
//!   Break (mine) + Place above. Aim with the centre crosshair, press to act.
//! - **Bottom-centre** — the 9-slot hotbar (drawn by `hud_ui`) with an
//!   Inventory button at its right end.
//! - **Top corners** — Pause/Chat (left), View/Zoom (right).
//! - Everything else is the look surface — drag to rotate the camera. The
//!   joystick and the look-drag use separate touch ids, so a left thumb moves
//!   while a right thumb (or any second finger) looks, simultaneously.
//!
//! ## Single source of truth for button rects
//!
//! [`layout_buttons`] returns each button's rectangle as a pure function of the
//! screen size. BOTH hit-testing ([`TouchInput::classify_zone`], in physical
//! px) and drawing ([`draw_touch_overlay`], in egui points) read it, so the
//! drawn button and its tappable zone can never drift apart — the failure mode
//! of the previous tiny-circle overlay, where the visible button and the hit
//! zone were computed independently.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::player_intent::PlayerIntent;

/// Platforms whose primary input is a touchscreen: the web build (phones,
/// tablets, touchscreen Chromebooks) and Android.
///
/// ONE const rather than a dozen copies of
/// `#[cfg(any(target_arch = "wasm32", target_os = "android"))]` scattered across
/// `lib.rs` and `game_loop.rs`. Touch needs every site to agree — event intake,
/// resize, the tick and frame intent merges, zoom, look, pause/chat buttons,
/// overlay draw, and per-frame edge clearing — and if one drifts you get a game
/// that draws controls it never reads, reads touches it never draws, or replays
/// every tap because the edges are never cleared.
///
/// A `bool` rather than a `cfg` because every touch code path COMPILES on every
/// platform (`touch_input` is target-agnostic; the OS text prompt lives in
/// `text_prompt.rs`, with a desktop backend that answers "cancelled"). `cfg!`
/// const-folds, so desktop pays nothing for the dead branch.
pub const TOUCH_PLATFORM: bool = cfg!(any(target_arch = "wasm32", target_os = "android"));

/// True where `text_prompt::request_text_prompt` can actually summon a soft
/// keyboard: the web (`window.prompt()`) and Android (the activity's text
/// dialog). On a touch device there, a menu text field is a tap-to-type button;
/// on desktop it stays an egui `TextEdit`.
pub const OS_KEYBOARD_PROMPT: bool = cfg!(any(target_arch = "wasm32", target_os = "android"));

/// Global "this is a touch device" flag, set once at WASM startup from
/// `navigator.maxTouchPoints` (and unconditionally by `android_main`). Read from places that can't reach a `TouchInput`
/// instance — the menu's text fields, which swap egui text entry (no soft
/// keyboard under winit+egui) for an OS-keyboard prompt on touch. Never changes
/// after startup, so `Relaxed` is fine.
static TOUCH_DEVICE: AtomicBool = AtomicBool::new(false);

/// Record whether this is a touch device. Called once at startup.
pub fn set_touch_device(v: bool) {
    TOUCH_DEVICE.store(v, Ordering::Relaxed);
}

/// True on a phone / tablet / touchscreen Chromebook (`maxTouchPoints > 0`).
pub fn is_touch_device() -> bool {
    TOUCH_DEVICE.load(Ordering::Relaxed)
}

/// Joystick travel that saturates movement, as a fraction of screen height (in
/// whatever unit the caller's height is). Scales the stick with the screen so
/// it stays thumb-sized on small phones and comfortable on a 15" Chromebook.
fn joy_radius(h: f32) -> f32 {
    (h * 0.12).max(55.0)
}

/// A button rectangle, in whatever coordinate space the caller passes —
/// physical px for hit-testing, egui points for drawing. Self-consistent
/// because [`layout_buttons`] is the single source for both.
#[derive(Clone, Copy, Debug)]
pub struct BtnRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl BtnRect {
    fn from_center(cx: f32, cy: f32, half: f32) -> Self {
        BtnRect {
            x0: cx - half,
            y0: cy - half,
            x1: cx + half,
            y1: cy + half,
        }
    }
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
    fn overlaps(&self, o: &BtnRect) -> bool {
        self.x0 < o.x1 && o.x0 < self.x1 && self.y0 < o.y1 && o.y0 < self.y1
    }
}

/// The on-screen buttons, in priority order (checked first-to-last in
/// `classify_zone`, so earlier entries win any seam overlap).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonKind {
    Jump,
    Sneak,
    Break,
    Place,
    Inventory,
    Pause,
    Chat,
    Perspective,
    Zoom,
}

/// Compute every button's rectangle for a `w`×`h` screen. Pure + proportional,
/// so it gives a consistent layout in either physical px (hit-test) or points
/// (draw). See the module docs for the layout.
///
/// `ppp` is how many units of THIS space make one egui point: `1.0` when `w`×`h`
/// are points (drawing), the pixels-per-point when they are physical px
/// (hit-testing). Every button but one is proportional to the screen and ignores
/// it; the Inventory button sits beside the drawn hotbar, whose size is fixed in
/// points (`hud_ui::hotbar_geom`), so it needs the scale to stay clear of it.
pub fn layout_buttons(w: f32, h: f32, ppp: f32) -> [(ButtonKind, BtnRect); 9] {
    let u = h * 0.13; // primary action-button side
    let r = u * 0.5;
    let m = h * 0.035; // margin from screen edges
    let gap = u * 0.16;

    // Bottom-right 2×2 action cluster. Right column (most thumb-reachable) =
    // Jump (lower) + Break (upper); left column = Sneak + Place.
    let xr = w - m - r;
    let xl = xr - (u + gap);
    let yl = h - m - r; // lower row
    let yu = yl - (u + gap); // upper row

    // Smaller corner buttons.
    let us = u * 0.74;
    let rs = us * 0.5;
    let ms = h * 0.028;

    let pause_c = (ms + rs, ms + rs);
    let chat_c = (ms + rs + us + gap * 0.7, ms + rs);
    let persp_c = (w - ms - rs, ms + rs);
    let zoom_c = (w - m - rs, h * 0.40);

    // Inventory sits just right of the DRAWN hotbar (its exact right edge, from
    // the one shared geometry), hugging the bottom. If that spot would run into
    // the Sneak/Place column (a squarer screen) it moves to the top row beside
    // Chat, which no hotbar can reach, so it never covers a slot.
    let ppp = ppp.max(0.5);
    let inv_side = u * 0.82;
    let inv_r = inv_side * 0.5;
    let hg = crate::hud_ui::hotbar_geom(0.0, 0.0, w / ppp, h / ppp);
    let inv_pad = u * 0.15;
    let inv_x0 = (hg.x + hg.total_w) * ppp + inv_pad;
    let inv_c = if inv_x0 + inv_side + inv_pad <= xl - r {
        (inv_x0 + inv_r, h - m * 0.9 - inv_r)
    } else {
        (chat_c.0 + rs + gap * 0.7 + inv_r, ms + inv_r)
    };

    [
        (ButtonKind::Jump, BtnRect::from_center(xr, yl, r)),
        (ButtonKind::Break, BtnRect::from_center(xr, yu, r)),
        (ButtonKind::Sneak, BtnRect::from_center(xl, yl, r)),
        (ButtonKind::Place, BtnRect::from_center(xl, yu, r)),
        (ButtonKind::Inventory, BtnRect::from_center(inv_c.0, inv_c.1, inv_r)),
        (ButtonKind::Pause, BtnRect::from_center(pause_c.0, pause_c.1, rs)),
        (ButtonKind::Chat, BtnRect::from_center(chat_c.0, chat_c.1, rs)),
        (ButtonKind::Perspective, BtnRect::from_center(persp_c.0, persp_c.1, rs)),
        (ButtonKind::Zoom, BtnRect::from_center(zoom_c.0, zoom_c.1, rs)),
    ]
}

/// Touch zone a press landed in.
enum TouchZone {
    Joystick,
    Look,
    Hotbar(usize),
    /// The off-hand box left of the hotbar (`hud_ui::HotbarGeom::offhand_box`).
    Offhand,
    Button(ButtonKind),
}

pub struct TouchInput {
    // Virtual joystick (own touch id).
    joystick_id: Option<u64>,
    joystick_origin: (f32, f32),
    joystick_current: (f32, f32),

    // Look drag (own touch id).
    look_id: Option<u64>,
    look_prev: (f32, f32),
    look_dx: f64,
    look_dy: f64,

    // Hold buttons track their OWN touch id so lifting another finger (e.g. the
    // joystick) doesn't release them — the bug in the previous overlay.
    break_id: Option<u64>,
    /// Off-hand release (slice 5) — the finger holding the Place button, so
    /// `place_held` (a raised shield) releases when ITS finger lifts.
    place_id: Option<u64>,
    sneak_id: Option<u64>,
    zoom_id: Option<u64>,
    /// The finger holding the Jump button, so `jump_down` lasts as long as it does.
    jump_id: Option<u64>,

    // Per-frame edge presses (consumed at end_frame) + holds.
    pub jump_pressed: bool,
    /// The Jump button is held down (a level, where `jump_pressed` is the
    /// one-frame edge). It is what `PlayerIntent::jump_held` carries, so holding
    /// the button swims up in water and keeps jumping on land, as Space does.
    pub jump_down: bool,
    pub break_held: bool,
    pub place_pressed: bool,
    /// The Place button is held down (a hold, where `place_pressed` is the
    /// one-frame edge): raises a shield (`PlayerIntent::use_held`).
    pub place_held: bool,
    pub sneak_held: bool,
    pub zoom_held: bool,
    pub pause_pressed: bool,
    pub chat_pressed: bool,
    pub inventory_pressed: bool,
    pub camera_cycle: bool,
    /// A tap on the off-hand box this frame (swap hands). Edge, cleared by `end_frame`.
    pub swap_hands: bool,
    pub hotbar_select: Option<usize>,

    // Screen size in PHYSICAL px (winit reports touch + size in physical px).
    screen_w: f32,
    screen_h: f32,
    // devicePixelRatio (egui points → physical px). Used to place the hotbar
    // hit-band at the SAME geometry `hud_ui` draws it, so a tapped slot is the
    // slot you see — the buttons already share `layout_buttons`; this gives the
    // hotbar the same single-source guarantee.
    pixels_per_point: f32,

    /// True while the player is dead, in a menu or in any panel: the buttons are
    /// hidden and a new touch is ignored (the menu owns it). See
    /// [`Self::set_suspended`].
    suspended: bool,

    /// Tripped on the first touch event. Latches the overlay on for the rest of
    /// the session.
    pub any_touch_seen: bool,
    /// Set once a real mouse motion is seen. Lets a desktop dev with a
    /// touchscreen dismiss the overlay until they actually tap — see
    /// [`Self::controls_visible`].
    mouse_seen: bool,
}

impl Default for TouchInput {
    fn default() -> Self {
        Self::new()
    }
}

impl TouchInput {
    pub fn new() -> Self {
        Self {
            joystick_id: None,
            joystick_origin: (0.0, 0.0),
            joystick_current: (0.0, 0.0),
            look_id: None,
            look_prev: (0.0, 0.0),
            look_dx: 0.0,
            look_dy: 0.0,
            break_id: None,
            place_id: None,
            sneak_id: None,
            zoom_id: None,
            jump_id: None,
            jump_pressed: false,
            jump_down: false,
            break_held: false,
            place_pressed: false,
            place_held: false,
            sneak_held: false,
            zoom_held: false,
            pause_pressed: false,
            chat_pressed: false,
            inventory_pressed: false,
            camera_cycle: false,
            swap_hands: false,
            hotbar_select: None,
            screen_w: 1280.0,
            screen_h: 720.0,
            pixels_per_point: 1.0,
            suspended: false,
            any_touch_seen: false,
            mouse_seen: false,
        }
    }

    pub fn set_screen_size(&mut self, w: f32, h: f32) {
        self.screen_w = w;
        self.screen_h = h;
    }

    /// Record the current devicePixelRatio (egui points → physical px) so the
    /// hotbar hit-band matches where `hud_ui` draws the slots.
    pub fn set_pixels_per_point(&mut self, ppp: f32) {
        self.pixels_per_point = ppp.max(0.5);
    }

    /// Screen size in physical px and the scale that draws the HUD, together.
    /// The caller reads the scale from the window (`scale_factor` times egui's
    /// zoom), NOT from egui's `pixels_per_point()`, which stays 1.0 until egui
    /// has run its first frame.
    pub fn set_display(&mut self, w: f32, h: f32, ppp: f32) {
        self.set_screen_size(w, h);
        self.set_pixels_per_point(ppp);
    }

    /// Hide the buttons and ignore new touches while the player is dead or a
    /// menu or panel is open, so a tap never reaches the world from behind it and
    /// nothing is drawn over the death screen. On either edge (open or close)
    /// every held finger, button and drag is dropped: a stick or button held
    /// across the change must not stay stuck, and a drag that started in a menu
    /// must not turn into a look when it closes (its finger has no id here, so
    /// its moves are ignored).
    pub fn set_suspended(&mut self, suspended: bool) {
        if self.suspended != suspended {
            self.release_all();
            self.suspended = suspended;
        }
    }

    /// Whether the buttons are hidden for a death screen, menu or panel.
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    /// Drop every finger, hold, edge and drag. Keeps the screen size, the scale
    /// and the "a touch was seen" latch.
    fn release_all(&mut self) {
        self.joystick_id = None;
        self.joystick_current = self.joystick_origin;
        self.look_id = None;
        self.look_dx = 0.0;
        self.look_dy = 0.0;
        self.break_id = None;
        self.place_id = None;
        self.sneak_id = None;
        self.zoom_id = None;
        self.jump_id = None;
        self.jump_pressed = false;
        self.jump_down = false;
        self.break_held = false;
        self.place_pressed = false;
        self.place_held = false;
        self.sneak_held = false;
        self.zoom_held = false;
        self.pause_pressed = false;
        self.chat_pressed = false;
        self.inventory_pressed = false;
        self.camera_cycle = false;
        self.swap_hands = false;
        self.hotbar_select = None;
    }

    /// Note a real mouse motion (so a desktop+touchscreen dev isn't shown the
    /// overlay until they tap).
    pub fn note_mouse(&mut self) {
        self.mouse_seen = true;
    }

    /// Whether to draw the touch overlay. Shows immediately on a touch device
    /// (so the controls are visible before the first tap — a Minecraft player
    /// expects to SEE them on load), unless a mouse has been used and no touch
    /// has happened yet. Any touch latches it on for good.
    pub fn controls_visible(&self) -> bool {
        !self.suspended && (self.any_touch_seen || (is_touch_device() && !self.mouse_seen))
    }

    fn classify_zone(&self, x: f32, y: f32) -> TouchZone {
        let w = self.screen_w.max(1.0);
        let h = self.screen_h.max(1.0);

        let ppp = self.pixels_per_point.max(0.5);

        // 1. Explicit button rects win (checked first, in priority order).
        for (kind, rect) in layout_buttons(w, h, ppp) {
            if rect.contains(x, y) {
                return TouchZone::Button(kind);
            }
        }

        // 2. Hotbar band (bottom-centre) — exact slot rects shared with the
        //    drawn hotbar via `hud_ui::hotbar_geom`, so the tapped slot is the
        //    one you see (the old hardcoded 0.31..0.69 fractions drifted off the
        //    fixed-width hotbar on wide screens). Web is single-player, so the
        //    viewport is the whole screen; points → physical px via ppp.
        let g = crate::hud_ui::hotbar_geom(0.0, 0.0, w / ppp, h / ppp);
        let x0 = g.x * ppp;
        let y0 = g.y * ppp;
        let slot_px = (g.slot + g.gap) * ppp;
        let total_px = g.total_w * ppp;
        // The off-hand box shares the hotbar's geometry (`offhand_box`), so the
        // box you tap is the one drawn; it beats the joystick quadrant it sits in.
        let (ox, oy, side) = g.offhand_box();
        let (ox, oy, side) = (ox * ppp, oy * ppp, side * ppp);
        if x >= ox && x <= ox + side && y >= oy && y <= oy + side {
            return TouchZone::Offhand;
        }
        // Tappable anywhere on the hotbar row down to the screen bottom.
        if y >= y0 && x >= x0 && x <= x0 + total_px {
            let slot = ((x - x0) / slot_px) as usize;
            return TouchZone::Hotbar(slot.min(8));
        }

        let rel_x = x / w;
        let rel_y = y / h;

        // 3. Joystick — bottom-left quadrant (leaves the top-left free for
        //    Pause/Chat and the centre free for look).
        if rel_x < 0.30 && rel_y > 0.45 {
            return TouchZone::Joystick;
        }

        // 4. Everything else looks.
        TouchZone::Look
    }

    pub fn on_touch_start(&mut self, id: u64, x: f32, y: f32) {
        self.any_touch_seen = true;
        if self.suspended {
            // A menu, panel or the death screen owns this finger.
            return;
        }
        match self.classify_zone(x, y) {
            TouchZone::Joystick => {
                self.joystick_id = Some(id);
                self.joystick_origin = (x, y);
                self.joystick_current = (x, y);
            }
            TouchZone::Look => {
                self.look_id = Some(id);
                self.look_prev = (x, y);
            }
            TouchZone::Hotbar(slot) => {
                self.hotbar_select = Some(slot);
            }
            TouchZone::Offhand => self.swap_hands = true,
            TouchZone::Button(kind) => match kind {
                ButtonKind::Jump => {
                    self.jump_pressed = true;
                    self.jump_down = true;
                    self.jump_id = Some(id);
                }
                ButtonKind::Place => {
                    self.place_pressed = true;
                    self.place_held = true;
                    self.place_id = Some(id);
                }
                ButtonKind::Inventory => self.inventory_pressed = true,
                ButtonKind::Pause => self.pause_pressed = true,
                ButtonKind::Chat => self.chat_pressed = true,
                ButtonKind::Perspective => self.camera_cycle = true,
                ButtonKind::Break => {
                    self.break_held = true;
                    self.break_id = Some(id);
                }
                ButtonKind::Sneak => {
                    self.sneak_held = true;
                    self.sneak_id = Some(id);
                }
                ButtonKind::Zoom => {
                    self.zoom_held = true;
                    self.zoom_id = Some(id);
                }
            },
        }
    }

    pub fn on_touch_move(&mut self, id: u64, x: f32, y: f32) {
        if self.joystick_id == Some(id) {
            self.joystick_current = (x, y);
        }
        if self.look_id == Some(id) {
            // Accumulate the RAW physical-px drag delta. Sensitivity + DPR
            // scaling are applied where it's consumed (`game_loop` rotates the
            // camera by `look_dx / pixels_per_point * sensitivity`), so a given
            // finger travel turns the view by the same amount at any
            // `devicePixelRatio` — the previous fixed `* 0.003` on physical px
            // made a high-DPR tablet 2–3× more sensitive than a 1× screen.
            self.look_dx += (x - self.look_prev.0) as f64;
            self.look_dy += (y - self.look_prev.1) as f64;
            self.look_prev = (x, y);
        }
    }

    pub fn on_touch_end(&mut self, id: u64) {
        if self.joystick_id == Some(id) {
            self.joystick_id = None;
            self.joystick_current = self.joystick_origin;
        }
        if self.look_id == Some(id) {
            self.look_id = None;
        }
        // Holds release only when THEIR finger lifts (not any finger).
        if self.break_id == Some(id) {
            self.break_held = false;
            self.break_id = None;
        }
        if self.place_id == Some(id) {
            self.place_held = false;
            self.place_id = None;
        }
        if self.sneak_id == Some(id) {
            self.sneak_held = false;
            self.sneak_id = None;
        }
        if self.zoom_id == Some(id) {
            self.zoom_held = false;
            self.zoom_id = None;
        }
        if self.jump_id == Some(id) {
            self.jump_down = false;
            self.jump_id = None;
        }
    }

    pub fn to_intent(&self) -> PlayerIntent {
        let max_radius = joy_radius(self.screen_h);
        let dx = self.joystick_current.0 - self.joystick_origin.0;
        let dy = self.joystick_current.1 - self.joystick_origin.1;
        // Push the stick near the edge to sprint (Bedrock "sprint using the
        // joystick" default).
        let mag = (dx * dx + dy * dy).sqrt() / max_radius;
        let sprint = self.joystick_id.is_some() && mag > 0.92;

        PlayerIntent {
            move_forward: (-dy / max_radius).clamp(-1.0, 1.0),
            move_right: (dx / max_radius).clamp(-1.0, 1.0),
            look_dx: self.look_dx,
            look_dy: self.look_dy,
            sprint,
            sneak: self.sneak_held,
            jump_held: self.jump_down,
            jump_pressed: self.jump_pressed,
            toggle_flight: false,
            break_block: self.break_held,
            place_block: self.place_pressed,
            use_held: self.place_held,
            guarding: false,
            toggle_inventory: self.inventory_pressed,
            drop_item: false, // BRIDGE: touch UI has no Q-equivalent yet
            pause: self.pause_pressed,
            hotbar_select: self.hotbar_select,
            scroll_delta: 0.0,
            toggle_debug: false,
            cursor_captured: true,
            rotate_ghost_ccw: false, // BRIDGE: touch UI has no rotate buttons yet
            rotate_ghost_cw: false,
            toggle_explorer: false, // BRIDGE: touch UI has no explorer button yet
            workshop_eyedropper: false, // BRIDGE: touch UI has no Workshop editor keys yet
            workshop_cycle_symmetry: false,
            workshop_pin: false, // BRIDGE: touch UI has no Workshop pin key yet
            workshop_gallery: false, // BRIDGE: touch UI has no Workshop Wardrobe key yet
            workshop_toggle_mode: false, // BRIDGE: touch UI has no Workshop mode key yet
            workshop_undo: false, // BRIDGE: touch UI has no Workshop undo key yet
            workshop_redo: false, // BRIDGE: ditto (Campaign S)
            workshop_picker: false, // BRIDGE: ditto
            workshop_paint_panel: false, // BRIDGE: ditto
            workshop_limbs: false, // BRIDGE: ditto
            workshop_tool_cycle: false, // BRIDGE: ditto
            workshop_fill: false, // BRIDGE: ditto
            workshop_grid: false, // BRIDGE: ditto
            shift_down: false, // BRIDGE: touch has no modifier key
            camera_cycle: self.camera_cycle,
            swap_hands: self.swap_hands,
            pick_block: false, // BRIDGE: no touch binding — keyboard + mouse only
        }
    }

    /// Accumulated look-drag delta in RAW physical px since the last
    /// `end_frame`. The consumer divides by `pixels_per_point` and applies the
    /// look sensitivity (touch has no pointer-lock, so it can't ride the mouse
    /// `cursor_captured` path).
    pub fn look_delta(&self) -> (f64, f64) {
        (self.look_dx, self.look_dy)
    }

    /// Take the "chat button tapped this frame" edge (cleared on read) — the
    /// caller pops the OS keyboard and submits a command, so it must read this
    /// before `end_frame`.
    pub fn take_chat_pressed(&mut self) -> bool {
        std::mem::take(&mut self.chat_pressed)
    }

    /// Take the "pause button tapped this frame" edge (cleared on read).
    pub fn take_pause_pressed(&mut self) -> bool {
        std::mem::take(&mut self.pause_pressed)
    }

    /// Clear per-frame edges. Call at end of each frame. Holds (jump / break /
    /// sneak / zoom) persist until their finger lifts.
    pub fn end_frame(&mut self) {
        self.look_dx = 0.0;
        self.look_dy = 0.0;
        self.jump_pressed = false;
        self.place_pressed = false;
        self.pause_pressed = false;
        self.chat_pressed = false;
        self.hotbar_select = None;
        self.inventory_pressed = false;
        self.camera_cycle = false;
        self.swap_hands = false;
    }
}

// ─────────────────────────── Overlay drawing ───────────────────────────────

/// Charcoal translucent fill + white icons = Minecraft Bedrock button style.
fn button_fill(active: bool) -> egui::Color32 {
    if active {
        egui::Color32::from_rgba_unmultiplied(70, 120, 165, 190)
    } else {
        egui::Color32::from_rgba_unmultiplied(18, 20, 28, 140)
    }
}

const ICON_COL: egui::Color32 = egui::Color32::from_rgba_premultiplied(238, 240, 248, 235);

fn draw_button(p: &egui::Painter, rect: egui::Rect, kind: ButtonKind, active: bool) {
    let radius = rect.height() * 0.22;
    p.rect_filled(rect, radius, button_fill(active));
    p.rect_stroke(
        rect,
        radius,
        egui::Stroke::new(1.5_f32, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 50)),
        egui::StrokeKind::Inside,
    );
    draw_icon(p, rect, kind);
}

fn draw_icon(p: &egui::Painter, rect: egui::Rect, kind: ButtonKind) {
    let c = rect.center();
    let i = rect.height() * 0.27; // icon half-extent
    let col = ICON_COL;
    match kind {
        ButtonKind::Jump => {
            let pts = vec![
                egui::pos2(c.x, c.y - i),
                egui::pos2(c.x - i * 0.95, c.y + i * 0.7),
                egui::pos2(c.x + i * 0.95, c.y + i * 0.7),
            ];
            p.add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
        }
        ButtonKind::Sneak => {
            let pts = vec![
                egui::pos2(c.x, c.y + i),
                egui::pos2(c.x - i * 0.95, c.y - i * 0.7),
                egui::pos2(c.x + i * 0.95, c.y - i * 0.7),
            ];
            p.add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
        }
        ButtonKind::Break => {
            // Pickaxe: a diagonal handle + a head stroke across the top.
            let st = egui::Stroke::new((i * 0.34).max(2.5), col);
            p.line_segment(
                [
                    egui::pos2(c.x - i * 0.7, c.y + i * 0.9),
                    egui::pos2(c.x + i * 0.45, c.y - i * 0.45),
                ],
                st,
            );
            p.line_segment(
                [
                    egui::pos2(c.x - i * 0.25, c.y - i * 0.95),
                    egui::pos2(c.x + i * 0.95, c.y - i * 0.05),
                ],
                st,
            );
        }
        ButtonKind::Place => {
            // A block: filled rounded square with a lighter top bevel.
            let q = i * 1.25;
            let fr = egui::Rect::from_center_size(c, egui::vec2(q, q));
            p.rect_filled(fr, q * 0.16, col);
            let top = egui::Rect::from_min_max(fr.left_top(), egui::pos2(fr.right(), fr.top() + q * 0.32));
            p.rect_filled(top, q * 0.16, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 70));
        }
        ButtonKind::Inventory => {
            // 2×2 grid of cells.
            let cell = i * 0.62;
            let step = cell * 0.78;
            for (gx, gy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let cc = egui::pos2(c.x + gx * step, c.y + gy * step);
                let rr = egui::Rect::from_center_size(cc, egui::vec2(cell, cell));
                p.rect_filled(rr, cell * 0.18, col);
            }
        }
        ButtonKind::Pause => {
            let bw = i * 0.38;
            let bh = i * 1.5;
            p.rect_filled(
                egui::Rect::from_center_size(egui::pos2(c.x - bw, c.y), egui::vec2(bw, bh)),
                1.5,
                col,
            );
            p.rect_filled(
                egui::Rect::from_center_size(egui::pos2(c.x + bw, c.y), egui::vec2(bw, bh)),
                1.5,
                col,
            );
        }
        ButtonKind::Chat => {
            let br = egui::Rect::from_center_size(egui::pos2(c.x, c.y - i * 0.15), egui::vec2(i * 1.9, i * 1.35));
            p.rect_filled(br, i * 0.45, col);
            // Little tail bottom-left.
            let pts = vec![
                egui::pos2(c.x - i * 0.55, br.bottom() - 1.0),
                egui::pos2(c.x - i * 0.95, br.bottom() + i * 0.6),
                egui::pos2(c.x - i * 0.05, br.bottom() - 1.0),
            ];
            p.add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
        }
        ButtonKind::Perspective => {
            let st = egui::Stroke::new((i * 0.22).max(1.5), col);
            p.circle_stroke(c, i * 0.62, st);
            p.circle_filled(c, i * 0.26, col);
        }
        ButtonKind::Zoom => {
            let st = egui::Stroke::new((i * 0.24).max(1.5), col);
            let cc = egui::pos2(c.x - i * 0.2, c.y - i * 0.2);
            p.circle_stroke(cc, i * 0.6, st);
            p.line_segment(
                [egui::pos2(cc.x + i * 0.45, cc.y + i * 0.45), egui::pos2(c.x + i * 0.85, c.y + i * 0.85)],
                st,
            );
        }
    }
}

/// Draw the touch control overlay via egui.
///
/// egui paints in *points*; touch coords + `screen_w/h` are physical px (winit
/// reports both in physical px). The button layout is proportional, so we
/// compute it directly in points here (`ctx.content_rect()`), and the matching
/// hit zones compute it in physical px — they line up at any
/// `devicePixelRatio`. The joystick origin is a physical-px touch coord, so it
/// is divided by `pixels_per_point` to land in points.
pub fn draw_touch_overlay(ctx: &egui::Context, touch: &TouchInput) {
    let ppp = ctx.pixels_per_point().max(1.0);
    let to_pt = |p: (f32, f32)| egui::pos2(p.0 / ppp, p.1 / ppp);
    let screen = ctx.content_rect();
    let sw = screen.width();
    let sh = screen.height();

    egui::Area::new(egui::Id::new("touch_overlay"))
        .fixed_pos(egui::pos2(0.0, 0.0))
        .interactable(false)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let p = ui.painter();
            let jr = joy_radius(sh);

            // The movement joystick: an always-visible, semi-transparent
            // two-ring stick in the bottom-left. The outer ring is the home
            // base (so the thumb knows where to land); the inner knob tracks the
            // finger and is clamped to the outer ring. A faint dark disc behind
            // the ring keeps it legible over bright terrain/sky.
            if touch.joystick_id.is_some() {
                // Active floating stick under the finger.
                let origin = to_pt(touch.joystick_origin);
                p.circle_filled(origin, jr, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 70));
                p.circle_stroke(
                    origin,
                    jr,
                    egui::Stroke::new(3.0_f32, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 180)),
                );
                let mut off = to_pt(touch.joystick_current) - origin;
                if off.length() > jr {
                    off = off.normalized() * jr;
                }
                p.circle_filled(origin + off, jr * 0.46, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 210));
            } else {
                // Idle "home" base — clearly visible so the player can see there
                // IS a stick to drag (faint enough to stay see-through).
                let anchor = egui::pos2(sw * 0.16, sh * 0.76);
                p.circle_filled(anchor, jr, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 55));
                p.circle_stroke(
                    anchor,
                    jr,
                    egui::Stroke::new(3.0_f32, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 130)),
                );
                p.circle_filled(anchor, jr * 0.42, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 110));
            }

            for (kind, rect) in layout_buttons(sw, sh, 1.0) {
                let r = egui::Rect::from_min_max(egui::pos2(rect.x0, rect.y0), egui::pos2(rect.x1, rect.y1));
                let active = match kind {
                    ButtonKind::Break => touch.break_held,
                    ButtonKind::Place => touch.place_held,
                    ButtonKind::Sneak => touch.sneak_held,
                    ButtonKind::Zoom => touch.zoom_held,
                    ButtonKind::Jump => touch.jump_down,
                    _ => false,
                };
                draw_button(p, r, kind, active);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_for(kind: ButtonKind, w: f32, h: f32) -> BtnRect {
        layout_buttons(w, h, 1.0)
            .into_iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, r)| r)
            .expect("button present")
    }

    /// Tap the visible centre of each button → it classifies as that button,
    /// across the screen sizes HappyOtter's Chromebook + common tablets/phones
    /// hit. This is the invariant the old overlay broke (draw ≠ hit zone).
    #[test]
    fn every_button_centre_hits_its_own_zone() {
        for (w, h) in [
            (1280.0, 720.0),
            (1920.0, 1080.0),
            (1366.0, 768.0),
            (2400.0, 1080.0), // phone landscape
            (1024.0, 768.0),  // 4:3 tablet
        ] {
            let mut t = TouchInput::new();
            t.set_screen_size(w, h);
            for (kind, rect) in layout_buttons(w, h, 1.0) {
                let cx = (rect.x0 + rect.x1) * 0.5;
                let cy = (rect.y0 + rect.y1) * 0.5;
                t.on_touch_start(1, cx, cy);
                let got = matches!(t.classify_zone(cx, cy), TouchZone::Button(k) if k == kind);
                assert!(got, "{kind:?} centre must classify as {kind:?} at {w}x{h}");
                t.on_touch_end(1);
            }
        }
    }

    /// No two button rectangles overlap (so a tap is never ambiguous).
    #[test]
    fn no_two_buttons_overlap() {
        for (w, h) in [(1280.0, 720.0), (1920.0, 1080.0), (2400.0, 1080.0)] {
            let buttons = layout_buttons(w, h, 1.0);
            for a in 0..buttons.len() {
                for b in (a + 1)..buttons.len() {
                    assert!(
                        !buttons[a].1.overlaps(&buttons[b].1),
                        "{:?} overlaps {:?} at {w}x{h}",
                        buttons[a].0,
                        buttons[b].0
                    );
                }
            }
        }
    }

    #[test]
    fn joystick_drives_movement_and_sprint() {
        let mut t = TouchInput::new();
        t.set_screen_size(1280.0, 720.0);
        // Plant in the bottom-left joystick zone.
        t.on_touch_start(1, 1280.0 * 0.12, 720.0 * 0.75);
        assert!(t.joystick_id.is_some(), "bottom-left tap grabs the joystick");
        // Push forward (up) past the sprint threshold.
        let r = joy_radius(720.0);
        t.on_touch_move(1, 1280.0 * 0.12, 720.0 * 0.75 - r);
        let intent = t.to_intent();
        assert!(intent.move_forward > 0.9, "push up → forward");
        assert!(intent.sprint, "edge of the stick sprints");
    }

    #[test]
    fn centre_drag_looks() {
        let mut t = TouchInput::new();
        t.set_screen_size(1280.0, 720.0);
        t.on_touch_start(1, 640.0, 360.0);
        assert!(t.look_id.is_some(), "centre tap is a look drag");
        t.on_touch_move(1, 700.0, 360.0);
        assert!(t.to_intent().look_dx > 0.0, "dragging right yaws right");
    }

    #[test]
    fn hotbar_band_selects_slots() {
        let mut t = TouchInput::new();
        t.set_screen_size(1280.0, 720.0);
        t.set_pixels_per_point(1.0);
        // Tap the centre of each drawn slot (shared geometry) → that slot.
        let g = crate::hud_ui::hotbar_geom(0.0, 0.0, 1280.0, 720.0);
        let centre = |i: usize| {
            (g.x + i as f32 * (g.slot + g.gap) + g.slot * 0.5, g.y + g.slot * 0.5)
        };
        for slot in [0usize, 4, 8] {
            let (cx, cy) = centre(slot);
            t.on_touch_start((slot + 1) as u64, cx, cy);
            assert_eq!(t.hotbar_select, Some(slot), "tap slot {slot} centre");
            t.on_touch_end((slot + 1) as u64);
        }
    }

    /// A tap inside the off-hand box (shared geometry with the HUD) swaps the
    /// hands for one frame, and is not a joystick, look or hotbar touch.
    #[test]
    fn a_tap_on_the_offhand_box_swaps_hands() {
        for (w, h) in [(1280.0, 720.0), (800.0, 360.0), (2400.0, 1080.0)] {
            let mut t = TouchInput::new();
            t.set_screen_size(w, h);
            t.set_pixels_per_point(1.0);
            let g = crate::hud_ui::hotbar_geom(0.0, 0.0, w, h);
            let (ox, oy, side) = g.offhand_box();
            t.on_touch_start(1, ox + side * 0.5, oy + side * 0.5);
            assert!(t.swap_hands, "{w}x{h}: the tap sets swap_hands");
            assert!(t.hotbar_select.is_none() && t.joystick_id.is_none() && t.look_id.is_none());
            assert!(t.to_intent().swap_hands && t.to_intent().cursor_captured, "→ the swap intent");
            t.end_frame();
            assert!(!t.to_intent().swap_hands, "an edge: cleared at the end of the frame");
        }
    }

    #[test]
    fn inventory_button_toggles_inventory() {
        let mut t = TouchInput::new();
        t.set_screen_size(1280.0, 720.0);
        let r = rect_for(ButtonKind::Inventory, 1280.0, 720.0);
        t.on_touch_start(1, (r.x0 + r.x1) * 0.5, (r.y0 + r.y1) * 0.5);
        assert!(t.inventory_pressed, "inventory button sets the press");
        assert!(t.to_intent().toggle_inventory, "→ toggle_inventory intent");
    }

    /// Off-hand release (slice 5) — the Place button is a hold as well as an edge:
    /// holding it raises a shield (`use_held`), and the one-frame `place_block` is
    /// unchanged (a held touch finger still places once, not every frame).
    #[test]
    fn holding_the_place_button_is_a_use_hold_until_its_finger_lifts() {
        let mut t = TouchInput::new();
        t.set_screen_size(1280.0, 720.0);
        let pr = rect_for(ButtonKind::Place, 1280.0, 720.0);
        t.on_touch_start(20, (pr.x0 + pr.x1) * 0.5, (pr.y0 + pr.y1) * 0.5);
        let first = t.to_intent();
        assert!(first.place_block && first.use_held, "the touch frame places once and holds");
        t.end_frame();
        let later = t.to_intent();
        assert!(!later.place_block, "the edge is one frame");
        assert!(later.use_held, "the hold stays while the finger is down");
        // Another finger lifting does not release it; its own finger does.
        t.on_touch_start(21, 1280.0 * 0.12, 720.0 * 0.75);
        t.on_touch_end(21);
        assert!(t.to_intent().use_held);
        t.on_touch_end(20);
        assert!(!t.to_intent().use_held, "released when its finger lifts");
    }

    #[test]
    fn break_hold_survives_a_second_finger_lifting() {
        // The previous overlay released break on ANY touch end. Now break holds
        // until ITS finger lifts, even if the joystick finger comes and goes.
        let mut t = TouchInput::new();
        t.set_screen_size(1280.0, 720.0);
        let br = rect_for(ButtonKind::Break, 1280.0, 720.0);
        t.on_touch_start(10, (br.x0 + br.x1) * 0.5, (br.y0 + br.y1) * 0.5);
        assert!(t.break_held);
        // A joystick finger goes down then up.
        t.on_touch_start(11, 1280.0 * 0.12, 720.0 * 0.75);
        t.on_touch_end(11);
        assert!(t.break_held, "break survives the joystick finger lifting");
        t.on_touch_end(10);
        assert!(!t.break_held, "break releases when ITS finger lifts");
    }

    #[test]
    fn sneak_hold_tracks_its_own_finger() {
        let mut t = TouchInput::new();
        t.set_screen_size(1280.0, 720.0);
        let sr = rect_for(ButtonKind::Sneak, 1280.0, 720.0);
        t.on_touch_start(3, (sr.x0 + sr.x1) * 0.5, (sr.y0 + sr.y1) * 0.5);
        assert!(t.sneak_held && t.to_intent().sneak);
        t.on_touch_end(99); // unrelated finger
        assert!(t.sneak_held, "sneak ignores other fingers");
        t.on_touch_end(3);
        assert!(!t.sneak_held);
    }

    #[test]
    fn controls_visible_on_touch_device_before_first_tap() {
        let mut t = TouchInput::new();
        set_touch_device(true);
        assert!(t.controls_visible(), "touch device shows controls on load");
        t.note_mouse();
        assert!(!t.controls_visible(), "a mouse user hides them until a tap");
        t.on_touch_start(1, 640.0, 360.0);
        assert!(t.controls_visible(), "any touch latches them back on");
        set_touch_device(false); // reset shared global for other tests
    }

    #[test]
    fn chat_and_pause_edges_are_taken_once() {
        let mut t = TouchInput::new();
        t.set_screen_size(1280.0, 720.0);
        let cr = rect_for(ButtonKind::Chat, 1280.0, 720.0);
        t.on_touch_start(1, (cr.x0 + cr.x1) * 0.5, (cr.y0 + cr.y1) * 0.5);
        assert!(t.take_chat_pressed(), "chat edge present");
        assert!(!t.take_chat_pressed(), "…and consumed");
        let pr = rect_for(ButtonKind::Pause, 1280.0, 720.0);
        t.on_touch_start(2, (pr.x0 + pr.x1) * 0.5, (pr.y0 + pr.y1) * 0.5);
        assert!(t.take_pause_pressed());
        assert!(!t.take_pause_pressed());
    }


    // ── Phone HUD: the drawn hotbar, the touch buttons and the labels ────────
    //
    // Everything below is a pure function or a `TouchInput` instance: no test
    // sets the process-global touch flag.

    /// The two real phone configurations (OnePlus 8T class): 2400x1080 physical
    /// px at 2.625 and at 3.0 pixels-per-point (about 914x411 and 800x360 points).
    const PHONES: [(f32, f32, f32); 2] = [(2400.0, 1080.0, 2.625), (2400.0, 1080.0, 3.0)];
    /// Plus a desktop-class window, so the phone fixes can't break it.
    /// Also the window less the 103 px camera-hole inset (2297 wide: the Inventory
    /// button sits within a couple of points of the Sneak/Place column there, and at
    /// the narrower ratio it takes the top-row fallback).
    const SIZES: [(f32, f32, f32); 7] = [
        (2400.0, 1080.0, 2.625),
        (2400.0, 1080.0, 3.0),
        (2297.0, 1080.0, 2.625),
        (2297.0, 1080.0, 3.0),
        (2200.0, 1080.0, 3.0), // narrower still: forces the top-row fallback
        (1024.0, 768.0, 1.0),  // 4:3 tablet
        (1280.0, 720.0, 1.0),
    ];

    /// Every rect the player sees at the bottom of the screen, in egui POINTS, at
    /// a physical size and scale: the nine drawn hotbar slots, the off-hand box,
    /// the touch buttons and the "NN free" label (a conservative 100x34 pt).
    struct DrawnHud {
        slots: Vec<BtnRect>,
        offhand: BtnRect,
        buttons: Vec<(ButtonKind, BtnRect)>,
        label: BtnRect,
    }

    fn drawn_hud(w_px: f32, h_px: f32, ppp: f32) -> DrawnHud {
        let (w, h) = (w_px / ppp, h_px / ppp);
        let g = crate::hud_ui::hotbar_geom(0.0, 0.0, w, h);
        let slots = (0..9)
            .map(|i| {
                let x0 = g.x + i as f32 * (g.slot + g.gap);
                BtnRect { x0, y0: g.y, x1: x0 + g.slot, y1: g.y + g.slot }
            })
            .collect();
        let (ox, oy, side) = g.offhand_box();
        let o = crate::hud_ui::vital_counts_origin(0.0, 0.0, w, h, 100.0, 34.0, true);
        DrawnHud {
            slots,
            offhand: BtnRect { x0: ox, y0: oy, x1: ox + side, y1: oy + side },
            buttons: layout_buttons(w, h, 1.0).to_vec(),
            label: BtnRect { x0: o.x, y0: o.y, x1: o.x + 100.0, y1: o.y + 34.0 },
        }
    }

    fn centre_of(r: &BtnRect) -> (f32, f32) {
        ((r.x0 + r.x1) * 0.5, (r.y0 + r.y1) * 0.5)
    }

    fn phone_touch(w_px: f32, h_px: f32, ppp: f32) -> TouchInput {
        let mut t = TouchInput::new();
        t.set_display(w_px, h_px, ppp);
        t
    }

    /// A tap at the centre of each DRAWN hotbar slot selects that slot, at both
    /// phone sizes and on desktop. The tap is in physical px (point x ppp).
    #[test]
    fn a_tap_on_each_drawn_hotbar_slot_selects_it() {
        for (w, h, ppp) in SIZES {
            let hud = drawn_hud(w, h, ppp);
            let mut t = phone_touch(w, h, ppp);
            for (i, slot) in hud.slots.iter().enumerate() {
                let (cx, cy) = centre_of(slot);
                t.on_touch_start(100 + i as u64, cx * ppp, cy * ppp);
                assert_eq!(t.hotbar_select, Some(i), "{w}x{h} @{ppp}: tap on drawn slot {i}");
                t.end_frame();
                t.on_touch_end(100 + i as u64);
            }
        }
    }

    /// Nothing drawn over the hotbar: no touch button covers a slot or the
    /// off-hand box, and nothing covers the "NN free" label.
    #[test]
    fn no_touch_button_covers_a_hotbar_slot_or_a_label() {
        for (w, h, ppp) in SIZES {
            let hud = drawn_hud(w, h, ppp);
            for (kind, b) in &hud.buttons {
                for (i, slot) in hud.slots.iter().enumerate() {
                    assert!(!b.overlaps(slot), "{w}x{h} @{ppp}: {kind:?} covers hotbar slot {i}");
                }
                assert!(!b.overlaps(&hud.offhand), "{w}x{h} @{ppp}: {kind:?} covers the off-hand box");
                assert!(!b.overlaps(&hud.label), "{w}x{h} @{ppp}: {kind:?} covers the 'NN free' label");
            }
        }
    }

    /// The buttons stay on screen and clear of each other at phone sizes (points).
    #[test]
    fn phone_buttons_are_on_screen_and_do_not_overlap() {
        for (w, h, ppp) in SIZES {
            let (wp, hp) = (w / ppp, h / ppp);
            let hud = drawn_hud(w, h, ppp);
            for (kind, b) in &hud.buttons {
                assert!(b.x0 >= 0.0 && b.y0 >= 0.0 && b.x1 <= wp && b.y1 <= hp, "{kind:?} off screen at {w}x{h} @{ppp}");
            }
            for a in 0..hud.buttons.len() {
                for b in (a + 1)..hud.buttons.len() {
                    assert!(!hud.buttons[a].1.overlaps(&hud.buttons[b].1), "{:?} overlaps {:?} at {w}x{h} @{ppp}", hud.buttons[a].0, hud.buttons[b].0);
                }
            }
        }
    }

    /// The tappable rect of a button is the drawn rect times the scale: draw and
    /// hit-test can't drift apart (the Inventory button depends on the scale).
    #[test]
    fn button_hit_rects_are_the_drawn_rects_scaled() {
        for (w, h, ppp) in SIZES {
            let drawn = layout_buttons(w / ppp, h / ppp, 1.0);
            let hit = layout_buttons(w, h, ppp);
            for ((k, d), (_, p)) in drawn.iter().zip(hit.iter()) {
                for (a, b) in [(d.x0, p.x0), (d.y0, p.y0), (d.x1, p.x1), (d.y1, p.y1)] {
                    assert!((a * ppp - b).abs() < 0.5, "{k:?} hit rect != drawn x scale at {w}x{h} @{ppp}");
                }
            }
        }
    }

    /// A tap on a drawn button's centre reaches that button at phone scale.
    #[test]
    fn every_button_centre_hits_its_own_zone_at_phone_scale() {
        for (w, h, ppp) in PHONES {
            let mut t = phone_touch(w, h, ppp);
            for (kind, rect) in drawn_hud(w, h, ppp).buttons {
                let (cx, cy) = centre_of(&rect);
                let got = matches!(t.classify_zone(cx * ppp, cy * ppp), TouchZone::Button(k) if k == kind);
                assert!(got, "{kind:?} centre must classify as {kind:?} at {w}x{h} @{ppp}");
            }
        }
    }

    /// The scale matters: with the stale 1.0 a phone's slot taps miss the drawn
    /// hotbar (the shipped failure). `lib.rs` now feeds the window's real scale.
    #[test]
    fn a_stale_scale_of_one_misses_the_drawn_hotbar() {
        let (w, h, ppp) = PHONES[0];
        let hud = drawn_hud(w, h, ppp);
        let mut t = phone_touch(w, h, 1.0);
        let (cx, cy) = centre_of(&hud.slots[4]);
        t.on_touch_start(1, cx * ppp, cy * ppp);
        assert_eq!(t.hotbar_select, None, "at scale 1.0 the tap falls through to look");
        t.set_display(w, h, ppp);
        t.end_frame();
        t.on_touch_end(1);
        t.on_touch_start(2, cx * ppp, cy * ppp);
        assert_eq!(t.hotbar_select, Some(4), "once the real scale is set the same tap selects slot 4");
    }

    /// A scale change after start moves the hit-band with the drawn hotbar.
    #[test]
    fn a_scale_change_after_start_moves_the_hit_band() {
        let mut t = phone_touch(2400.0, 1080.0, 2.625);
        t.set_display(2400.0, 1080.0, 3.0);
        let hud = drawn_hud(2400.0, 1080.0, 3.0);
        let (cx, cy) = centre_of(&hud.slots[8]);
        t.on_touch_start(1, cx * 3.0, cy * 3.0);
        assert_eq!(t.hotbar_select, Some(8));
    }

    /// Holding the Jump button holds `jump_held` for as long as the finger is
    /// down (swims up in water, keeps jumping on land, like Space), and the
    /// one-frame `jump_pressed` edge is still an edge.
    #[test]
    fn holding_touch_jump_holds_jump_until_its_finger_lifts() {
        let mut t = phone_touch(2400.0, 1080.0, 2.625);
        let jr = drawn_hud(2400.0, 1080.0, 2.625)
            .buttons
            .into_iter()
            .find(|(k, _)| *k == ButtonKind::Jump)
            .unwrap()
            .1;
        let (cx, cy) = centre_of(&jr);
        t.on_touch_start(7, cx * 2.625, cy * 2.625);
        assert!(t.to_intent().jump_held, "the frame it goes down");
        // 30 frames later (about half a second) the finger is still down.
        for _ in 0..30 {
            t.end_frame();
        }
        let held = t.to_intent();
        assert!(held.jump_held, "still held after many frames (a level, not an edge)");
        assert!(!held.jump_pressed, "the press itself stays a one-frame edge");
        // Another finger lifting does not release it.
        t.on_touch_start(8, 100.0, 900.0);
        t.on_touch_end(8);
        assert!(t.to_intent().jump_held);
        t.on_touch_end(7);
        assert!(!t.to_intent().jump_held, "released when ITS finger lifts");
    }

    fn press_everything(t: &mut TouchInput, w: f32, h: f32, ppp: f32) {
        // joystick, look, jump, break, place, sneak each on their own finger.
        t.on_touch_start(1, w * 0.10, h * 0.75);
        t.on_touch_move(1, w * 0.10, h * 0.55);
        t.on_touch_start(2, w * 0.50, h * 0.30);
        t.on_touch_move(2, w * 0.55, h * 0.30);
        for (id, kind) in [
            (3u64, ButtonKind::Jump),
            (4, ButtonKind::Break),
            (5, ButtonKind::Place),
            (6, ButtonKind::Sneak),
        ] {
            let r = drawn_hud(w, h, ppp).buttons.into_iter().find(|(k, _)| *k == kind).unwrap().1;
            let (cx, cy) = centre_of(&r);
            t.on_touch_start(id, cx * ppp, cy * ppp);
        }
    }

    /// Opening a menu (or dying) drops every held finger: nothing stays stuck,
    /// and nothing leaks into the intent while the menu is up or once it closes.
    #[test]
    fn a_menu_opening_releases_every_held_touch() {
        let (w, h, ppp) = PHONES[0];
        let mut t = phone_touch(w, h, ppp);
        press_everything(&mut t, w, h, ppp);
        let before = t.to_intent();
        assert!(before.jump_held && before.break_block && before.use_held && before.sneak && before.move_forward > 0.5);

        t.set_suspended(true);
        let i = t.to_intent();
        assert!(!i.jump_held && !i.break_block && !i.use_held && !i.sneak, "buttons released");
        assert!(i.move_forward == 0.0 && i.move_right == 0.0, "stick released");
        assert!(i.look_dx == 0.0 && i.look_dy == 0.0, "look dropped");
        assert!(!t.controls_visible(), "buttons are hidden behind the menu");

        // Lifting those fingers while the menu is up changes nothing.
        for id in 1..=6 {
            t.on_touch_end(id);
        }
        t.set_suspended(false);
        let after = t.to_intent();
        assert!(!after.jump_held && !after.break_block && !after.use_held && !after.sneak);
        assert!(after.move_forward == 0.0 && after.look_dx == 0.0);
    }

    /// A drag that starts while a menu is open must not become a camera look
    /// when the menu closes under the still-moving finger, and a button pressed
    /// in the menu's place must not latch.
    #[test]
    fn a_drag_started_in_a_menu_never_becomes_a_look() {
        let (w, h, ppp) = PHONES[0];
        let mut t = phone_touch(w, h, ppp);
        t.set_suspended(true);
        t.on_touch_start(1, w * 0.5, h * 0.3); // would be a look drag
        t.on_touch_move(1, w * 0.6, h * 0.4);
        assert_eq!(t.look_delta(), (0.0, 0.0), "no look while the menu is up");
        t.set_suspended(false); // the menu closes; the finger is still down
        t.on_touch_move(1, w * 0.7, h * 0.5);
        t.on_touch_move(1, w * 0.8, h * 0.6);
        assert_eq!(t.look_delta(), (0.0, 0.0), "the old finger never turns the camera");
        t.on_touch_end(1);
        assert!(!t.to_intent().jump_held && !t.to_intent().break_block);
        // A fresh touch after the menu closed works as normal.
        t.on_touch_start(2, w * 0.5, h * 0.3);
        t.on_touch_move(2, w * 0.55, h * 0.3);
        assert!(t.look_delta().0 > 0.0, "new touches look normally");
    }

    /// A button pressed on the frame a menu opens does not fire in the menu.
    #[test]
    fn touches_are_ignored_while_suspended() {
        let (w, h, ppp) = PHONES[1];
        let mut t = phone_touch(w, h, ppp);
        t.set_suspended(true);
        press_everything(&mut t, w, h, ppp);
        let i = t.to_intent();
        assert!(!i.jump_held && !i.jump_pressed && !i.break_block && !i.place_block && !i.use_held && !i.sneak);
        assert!(t.hotbar_select.is_none() && !t.inventory_pressed && !t.pause_pressed);
        // …but the "a touch happened" latch still turns on (a first tap on a card).
        assert!(t.any_touch_seen);
    }

    /// The death panel is centred on the screen at phone scale (it was centred
    /// as if physical px were points, which pushed it to the right edge).
    #[test]
    fn the_death_panel_centres_on_the_screen_in_points() {
        for (w, h, ppp) in PHONES {
            let vp = crate::screen::ViewportRect::full(w as u32, h as u32);
            let c = crate::hud_ui::death_panel_centre(&vp, ppp);
            assert!((c.x - w / ppp / 2.0).abs() < 0.5 && (c.y - h / ppp / 2.0).abs() < 0.5, "{w}x{h} @{ppp}: centre {c:?}");
        }
    }

    /// Desktop keeps the label where it was: the viewport's right edge, 12 in.
    #[test]
    fn the_free_label_stays_put_on_desktop() {
        let o = crate::hud_ui::vital_counts_origin(0.0, 0.0, 1280.0, 720.0, 60.0, 16.0, false);
        assert!((o.x - (1280.0 - 12.0 - 60.0)).abs() < 0.01);
        assert!((o.y - (720.0 - 16.0 - 48.0 - 6.0 - 16.0)).abs() < 0.01);
    }
}

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

/// Global "this is a touch device" flag, set once at WASM startup from
/// `navigator.maxTouchPoints`. Read from places that can't reach a `TouchInput`
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

/// Pop the OS soft keyboard for a single line of text and return what was
/// typed (`None` on cancel / native). `window.prompt()` is the one reliable way
/// to summon the on-screen keyboard on a tablet/Chromebook with no hardware
/// keyboard — winit+egui (we don't use eframe's hidden text-agent) won't bring
/// it up on its own. Used for the world-name field and the chat/command line.
#[cfg(target_arch = "wasm32")]
pub fn os_keyboard_prompt(message: &str, default: &str) -> Option<String> {
    web_sys::window()?
        .prompt_with_message_and_default(message, default)
        .ok()
        .flatten()
}

/// Native stub — there's always a hardware keyboard off-web.
#[cfg(not(target_arch = "wasm32"))]
pub fn os_keyboard_prompt(_message: &str, _default: &str) -> Option<String> {
    None
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
pub fn layout_buttons(w: f32, h: f32) -> [(ButtonKind, BtnRect); 9] {
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

    // Inventory sits just right of the centred hotbar, hugging the bottom.
    let inv_side = u * 0.82;
    let inv_r = inv_side * 0.5;
    let inv_c = (w * 0.715, h - m * 0.9 - inv_r);

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
    sneak_id: Option<u64>,
    zoom_id: Option<u64>,

    // Per-frame edge presses (consumed at end_frame) + holds.
    pub jump_pressed: bool,
    pub break_held: bool,
    pub place_pressed: bool,
    pub sneak_held: bool,
    pub zoom_held: bool,
    pub pause_pressed: bool,
    pub chat_pressed: bool,
    pub inventory_pressed: bool,
    pub camera_cycle: bool,
    pub hotbar_select: Option<usize>,

    // Screen size in PHYSICAL px (winit reports touch + size in physical px).
    screen_w: f32,
    screen_h: f32,
    // devicePixelRatio (egui points → physical px). Used to place the hotbar
    // hit-band at the SAME geometry `hud_ui` draws it, so a tapped slot is the
    // slot you see — the buttons already share `layout_buttons`; this gives the
    // hotbar the same single-source guarantee.
    pixels_per_point: f32,

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
            sneak_id: None,
            zoom_id: None,
            jump_pressed: false,
            break_held: false,
            place_pressed: false,
            sneak_held: false,
            zoom_held: false,
            pause_pressed: false,
            chat_pressed: false,
            inventory_pressed: false,
            camera_cycle: false,
            hotbar_select: None,
            screen_w: 1280.0,
            screen_h: 720.0,
            pixels_per_point: 1.0,
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
        self.any_touch_seen || (is_touch_device() && !self.mouse_seen)
    }

    fn classify_zone(&self, x: f32, y: f32) -> TouchZone {
        let w = self.screen_w.max(1.0);
        let h = self.screen_h.max(1.0);

        // 1. Explicit button rects win (checked first, in priority order).
        for (kind, rect) in layout_buttons(w, h) {
            if rect.contains(x, y) {
                return TouchZone::Button(kind);
            }
        }

        // 2. Hotbar band (bottom-centre) — exact slot rects shared with the
        //    drawn hotbar via `hud_ui::hotbar_geom`, so the tapped slot is the
        //    one you see (the old hardcoded 0.31..0.69 fractions drifted off the
        //    fixed-width hotbar on wide screens). Web is single-player, so the
        //    viewport is the whole screen; points → physical px via ppp.
        let ppp = self.pixels_per_point.max(0.5);
        let g = crate::hud_ui::hotbar_geom(0.0, 0.0, w / ppp, h / ppp);
        let x0 = g.x * ppp;
        let y0 = g.y * ppp;
        let slot_px = (g.slot + g.gap) * ppp;
        let total_px = g.total_w * ppp;
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
            TouchZone::Button(kind) => match kind {
                ButtonKind::Jump => self.jump_pressed = true,
                ButtonKind::Place => self.place_pressed = true,
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
        if self.sneak_id == Some(id) {
            self.sneak_held = false;
            self.sneak_id = None;
        }
        if self.zoom_id == Some(id) {
            self.zoom_held = false;
            self.zoom_id = None;
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
            jump_held: false,
            jump_pressed: self.jump_pressed,
            toggle_flight: false,
            break_block: self.break_held,
            place_block: self.place_pressed,
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

    /// Clear per-frame edges. Call at end of each frame. Holds (break / sneak /
    /// zoom) persist until their finger lifts.
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

            for (kind, rect) in layout_buttons(sw, sh) {
                let r = egui::Rect::from_min_max(egui::pos2(rect.x0, rect.y0), egui::pos2(rect.x1, rect.y1));
                let active = match kind {
                    ButtonKind::Break => touch.break_held,
                    ButtonKind::Sneak => touch.sneak_held,
                    ButtonKind::Zoom => touch.zoom_held,
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
        layout_buttons(w, h)
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
            for (kind, rect) in layout_buttons(w, h) {
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
            let buttons = layout_buttons(w, h);
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

    #[test]
    fn inventory_button_toggles_inventory() {
        let mut t = TouchInput::new();
        t.set_screen_size(1280.0, 720.0);
        let r = rect_for(ButtonKind::Inventory, 1280.0, 720.0);
        t.on_touch_start(1, (r.x0 + r.x1) * 0.5, (r.y0 + r.y1) * 0.5);
        assert!(t.inventory_pressed, "inventory button sets the press");
        assert!(t.to_intent().toggle_inventory, "→ toggle_inventory intent");
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
}

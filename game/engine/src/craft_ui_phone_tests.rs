//! The inventory window on a phone, drawn headless (egui is pure CPU) with the phone's own
//! input (`touch_events`), style (`configure_style_for(.., true)`), scale (`pixels_per_point`)
//! and viewport (physical pixels, as the game passes it): every slot is on the screen at once,
//! a tap then a tap moves an item, and a desktop window keeps the layout it always had.
use super::*;
use crate::egui_integration::{configure_style_for, touch_events};
use crate::screen::ViewportRect;

/// The OnePlus 8T of the 0.2.37 device test (2400 x 1080, 3.0 points per pixel), and the same
/// panel at the 2.625 a Pixel 8 reports. (Screen size in points, pixels per point.)
const DEVICES: [(egui::Vec2, f32); 2] = [(egui::vec2(800.0, 360.0), 3.0), (egui::vec2(914.0, 411.0), 2.625)];
/// The smallest a slot may be drawn on a phone, in points.
const THUMB: f32 = 28.0;

fn screen_of(size: egui::Vec2) -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, size)
}

/// Every piece of text a frame drew, with where it sits.
fn texts_of(output: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
    fn walk(shape: &egui::epaint::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::epaint::Shape::Text(t) => out.push((t.galley.text().to_string(), t.galley.rect.translate(t.pos.to_vec2()))),
            egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    output.shapes.iter().for_each(|clipped| walk(&clipped.shape, &mut out));
    out
}

struct Rig {
    ctx: egui::Context,
    size: egui::Vec2,
    ppp: f32,
    ui_state: CraftingUi,
    inventory: Inventory,
    armour: [Option<ArmourItem>; 4],
    registry: BlockRegistry,
    output: egui::FullOutput,
}

impl Rig {
    fn new(size: egui::Vec2, ppp: f32, touch: bool) -> Rig {
        let ctx = egui::Context::default();
        if touch {
            configure_style_for(&ctx, true);
        }
        ctx.set_pixels_per_point(ppp);
        let mut rig = Rig {
            ctx,
            size,
            ppp,
            ui_state: CraftingUi::new(),
            inventory: Inventory::new(),
            armour: [None, None, None, None],
            registry: BlockRegistry::new(),
            output: Default::default(),
        };
        // An opening Area spends a few frames sizing itself.
        for _ in 0..8 {
            rig.frame(vec![]);
        }
        rig
    }

    /// What the game passes: the viewport in physical pixels.
    fn viewport(&self) -> ViewportRect {
        ViewportRect::full((self.size.x * self.ppp).round() as u32, (self.size.y * self.ppp).round() as u32)
    }

    /// One frame, and what the game does with the click it returns.
    fn frame(&mut self, events: Vec<egui::Event>) -> ClickTarget {
        let viewport = self.viewport();
        let input = egui::RawInput { screen_rect: Some(screen_of(self.size)), events, ..Default::default() };
        let mut target = ClickTarget::None;
        let Rig { ctx, ui_state, inventory, armour, registry, .. } = self;
        let output = ctx.run(input, |ctx| {
            target = draw_crafting_ui(ctx, &viewport, 0, ui_state, inventory, armour, crate::hud_ui::OffhandBox::Empty, registry, &NoIcons);
            // The carried item is drawn by its own call in the game loop.
            paint_carried_item(ctx, &viewport, 0, ui_state.cursor_item.as_ref(), None, false, registry, &NoIcons);
        });
        self.output = output;
        if let ClickTarget::InventorySlot(slot, right) = target {
            self.ui_state.click_inventory_slot(slot, &mut self.inventory, right);
        }
        target
    }

    /// Play a finger gesture and return every click it made.
    fn play(&mut self, frames: Vec<Vec<egui::Event>>) -> Vec<ClickTarget> {
        let mut clicks: Vec<ClickTarget> = frames.into_iter().map(|events| self.frame(events)).collect();
        clicks.push(self.frame(vec![]));
        clicks
    }

    fn tap(&mut self, at: egui::Pos2) -> Vec<ClickTarget> {
        self.play(touch_events::tap(at))
    }

    /// Every slot drawn this frame: its rectangle and the clip it was drawn under.
    fn slots(&self) -> Vec<(egui::Rect, egui::Rect)> {
        fn walk(shape: &egui::epaint::Shape, clip: egui::Rect, out: &mut Vec<(egui::Rect, egui::Rect)>) {
            match shape {
                egui::epaint::Shape::Rect(r) if r.fill == SLOT_BG && (r.rect.width() - r.rect.height()).abs() < 0.5 => out.push((r.rect, clip)),
                egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        self.output.shapes.iter().for_each(|clipped| walk(&clipped.shape, clipped.clip_rect, &mut out));
        out
    }

    fn on_screen(&self, label: &str) -> bool {
        texts_of(&self.output).iter().any(|(t, r)| t == label && screen_of(self.size).contains_rect(*r))
    }

    /// Where each inventory slot (hotbar 0..9, bag 9..36) is drawn, found by tapping every drawn
    /// slot once on an empty bag (an empty slot under an empty hand does nothing).
    fn inventory_slot_rects(&mut self) -> std::collections::BTreeMap<usize, egui::Rect> {
        assert!(self.inventory.slots_iter().all(|s| s.is_none()), "map the slots on an empty bag");
        let mut map = std::collections::BTreeMap::new();
        for (rect, _) in self.slots() {
            for click in self.tap(rect.center()) {
                if let ClickTarget::InventorySlot(i, _) = click {
                    map.insert(i, rect);
                }
            }
        }
        map
    }
}

#[test]
fn every_slot_of_the_inventory_is_on_the_screen_at_once_on_a_phone() {
    for (size, ppp) in DEVICES {
        let rig = Rig::new(size, ppp, true);
        let slots = rig.slots();
        // 4 armour + the off-hand + the 2 x 2 grid + its output + 27 bag slots + 9 hotbar slots.
        assert!(slots.len() >= 46, "only {} slots were drawn at {size:?}", slots.len());
        for (rect, clip) in &slots {
            assert!(screen_of(size).contains_rect(*rect), "a slot {rect:?} is off the {size:?} screen");
            assert!(clip.contains_rect(*rect), "a slot {rect:?} is cut off by its clip {clip:?} at {size:?}");
            assert!(rect.width() >= THUMB, "a slot is only {} points wide at {size:?}", rect.width());
        }
        for label in ["\u{2195} Sort", "\u{1F5D1} Trash", "\u{1F4D6} Recipe Book"] {
            assert!(rig.on_screen(label), "{label:?} is not on the {size:?} screen: {:?}", texts_of(&rig.output));
        }
    }
}

#[test]
fn a_tap_on_a_slot_then_a_tap_on_another_moves_the_item_on_a_phone() {
    for (size, ppp) in DEVICES {
        let mut rig = Rig::new(size, ppp, true);
        let map = rig.inventory_slot_rects();
        assert_eq!(map.keys().copied().collect::<Vec<_>>(), (0..36).collect::<Vec<_>>(), "a slot of the bag or hotbar cannot be tapped at {size:?}");
        let stone = ItemStack::new_block(crate::block::STONE, 5);
        rig.inventory.set_slot(9, Some(stone.clone()));
        rig.tap(map[&9].center());
        assert_eq!(rig.ui_state.cursor_item.as_ref(), Some(&stone), "the first tap did not pick the stack up at {size:?}");
        assert!(rig.inventory.slot(9).is_none());
        rig.tap(map[&20].center());
        assert_eq!(rig.inventory.slot(20), Some(&stone), "the second tap did not put it down at {size:?}");
        assert!(rig.ui_state.cursor_item.is_none() && rig.inventory.slot(9).is_none());
        // ...and to the hotbar.
        rig.tap(map[&20].center());
        rig.tap(map[&3].center());
        assert_eq!(rig.inventory.slot(3), Some(&stone), "the item did not reach the hotbar at {size:?}");
    }
}

#[test]
fn a_finger_that_drifts_a_few_points_while_tapping_still_taps() {
    for (size, ppp) in DEVICES {
        let mut rig = Rig::new(size, ppp, true);
        let map = rig.inventory_slot_rects();
        let stone = ItemStack::new_block(crate::block::STONE, 5);
        rig.inventory.set_slot(20, Some(stone.clone()));
        let at = map[&20].center();
        let drift = at + egui::vec2(9.0, 0.0);
        rig.play(vec![touch_events::start(at), touch_events::moved(drift), touch_events::end(drift)]);
        assert_eq!(rig.ui_state.cursor_item.as_ref(), Some(&stone), "a tap that drifted 9 points did not pick the stack up at {size:?}");
    }
}

#[test]
fn the_carried_item_is_drawn_on_the_phone_screen_with_no_finger_down() {
    for (size, ppp) in DEVICES {
        let mut rig = Rig::new(size, ppp, true);
        let map = rig.inventory_slot_rects();
        rig.inventory.set_slot(9, Some(ItemStack::new_block(crate::block::STONE, 5)));
        rig.tap(map[&9].center());
        let label = texts_of(&rig.output).into_iter().find(|(t, _)| t.contains("(5)")).unwrap_or_else(|| panic!("no carried-item label at {size:?}: {:?}", texts_of(&rig.output)));
        assert!(screen_of(size).contains_rect(label.1), "the carried item's label {:?} is off the {size:?} screen", label.1);
    }
}

#[test]
fn a_desktop_window_keeps_the_inventory_layout_it_always_had() {
    let mut rig = Rig::new(egui::vec2(1280.0, 720.0), 1.0, false);
    for _ in 0..4 {
        rig.frame(vec![]);
    }
    let slots = rig.slots();
    assert!(slots.len() >= 46);
    assert!(slots.iter().all(|(r, _)| (r.width() - 48.0).abs() < 0.01), "a desktop slot is not 48 points: {:?}", slots.iter().map(|(r, _)| r.width()).collect::<Vec<_>>());
    // Nine to a row, 52 points apart (a 48 slot and a 4 gap).
    let mut rows: std::collections::BTreeMap<i32, Vec<f32>> = Default::default();
    for (r, _) in &slots {
        rows.entry(r.top().round() as i32).or_default().push(r.left());
    }
    let full: Vec<&Vec<f32>> = rows.values().filter(|row| row.len() == 9).collect();
    assert_eq!(full.len(), 4, "the bag's three rows and the hotbar should each hold nine slots: {rows:?}");
    for row in full {
        let mut xs = row.clone();
        xs.sort_by(f32::total_cmp);
        assert!(xs.windows(2).all(|w| (w[1] - w[0] - 52.0).abs() < 0.01), "the slots of a row are not 52 apart: {xs:?}");
    }
}

//! Spec 48 — Electricity crosshair hover label.
//!
//! Mirrors `campfire_ui`'s hover label: when the player's crosshair targets any
//! power/logic block within strike distance, this overlay shows the device's
//! current state — fuel/charge/turning/on-off — without opening a UI. This was
//! the deferred "generator fuel-gauge panel" polish item from the Spec 48 row
//! in `docs/foundations/README.md`, generalised to every device kind rather
//! than the Steam Generator alone. Pure render — no state mutation; reads
//! `PowerDeviceData` through the world.

use crate::block::{BlockId, BlockRegistry};
use crate::power::{GateOp, PowerDeviceData, PowerDeviceKind, BATTERY_CAPACITY};
use crate::screen::ViewportRect;

/// Ticks-to-seconds, rounded UP so a device with fuel/charge remaining never
/// reads "0 s left" the tick before it actually runs out.
fn ceil_secs(ticks: u32) -> u32 {
    ticks.div_ceil(20)
}

/// AND/OR/NOT/XOR, matching the player-guide's short-hand.
fn gate_op_name(op: GateOp) -> &'static str {
    match op {
        GateOp::And => "AND",
        GateOp::Or => "OR",
        GateOp::Not => "NOT",
        GateOp::Xor => "XOR",
    }
}

/// What the Windmill's hover label needs beyond the device itself: the wind at
/// the mill's own height, and whether it is standing out in the open. Both are
/// world queries this pure-render module can't make, so the caller hands them
/// over the way it already hands over `fed`. Every other device kind ignores it.
#[derive(Clone, Copy, Debug)]
pub struct WindmillView {
    /// Wind speed at the mill (already lifted by `wind::with_altitude`).
    pub speed: f32,
    /// `power::windmill_is_exposed` — clear sky above, elbow room beside.
    pub exposed: bool,
}

/// A derived `Default` would have made `exposed` FALSE, and "blocked (needs
/// open sky)" is the one Windmill line that accuses the builder of a mistake.
/// Every non-Windmill call site passes the default purely to fill the argument,
/// so the default must be the benign reading, not the failure state: a caller
/// that forgets to compute exposure gets "still / turning" rather than sending
/// a player off to dig out a mill that was never walled in.
impl Default for WindmillView {
    fn default() -> Self {
        Self { speed: 0.0, exposed: true }
    }
}

/// Pure: compute the crosshair hover label's text for a power device. `block`
/// is the targeted block id (the render-facing source of truth for the
/// lit/turning twin — mirrors `campfire_hover_text`'s `lit_block` bool);
/// `fed` is whether the device is currently being driven by an adjacent
/// source/cable (used only by Battery, to show "charging"); `windmill` is the
/// wind/exposure read-out (used only by Windmill).
///
/// One or two short, UK-English lines, no money/earning words — a kid
/// looking at a device sees what it's doing without opening anything.
pub fn power_hover_text(
    device: &PowerDeviceData,
    block: BlockId,
    fed: bool,
    windmill: WindmillView,
) -> String {
    match device.kind {
        PowerDeviceKind::SteamGenerator => {
            let burning = block == crate::block::STEAM_GENERATOR_LIT;
            if burning {
                let fuel = device.fuel.as_ref();
                let secs = ceil_secs(fuel.map(|f| f.fuel_ticks_remaining).unwrap_or(0));
                let mut line = format!("Steam Generator — burning, about {secs} s left");
                if let Some(queued) = fuel.and_then(|f| f.fuel.as_ref()).map(|s| s.count)
                    && queued > 0
                {
                    line.push_str(&format!(" (+ {queued} more)"));
                }
                line
            } else {
                "Steam Generator — cold. Right-click with fuel.".to_string()
            }
        }
        PowerDeviceKind::Battery => {
            let pct = (device.charge.saturating_mul(100) / BATTERY_CAPACITY.max(1)).min(100);
            match (device.charge, fed) {
                (0, true) => "Battery — empty, charging".to_string(),
                (0, false) => "Battery — empty".to_string(),
                (_, true) => format!("Battery — {pct} % charge, charging"),
                (_, false) => format!("Battery — {pct} % charge"),
            }
        }
        PowerDeviceKind::HandCrank => {
            if device.charge > 0 {
                format!("Hand Crank — turning, {} s left", ceil_secs(device.charge))
            } else {
                "Hand Crank — still. Right-click to crank.".to_string()
            }
        }
        PowerDeviceKind::WaterWheel => {
            let turning = block == crate::block::WATER_WHEEL_TURNING;
            if turning {
                "Water Wheel — turning".to_string()
            } else {
                "Water Wheel — still. It needs a stream, not a pond.".to_string()
            }
        }
        PowerDeviceKind::Windmill => {
            // "blocked" comes first: a walled-in mill is the one failure a
            // builder can actually fix, and telling them the wind is fresh
            // while they stand in a cellar is no help at all.
            if !windmill.exposed {
                "Windmill — blocked (needs open sky)".to_string()
            } else {
                let word = crate::wind::word(windmill.speed);
                let state =
                    if block == crate::block::WINDMILL_TURNING { "turning" } else { "still" };
                format!("Windmill — {state} (wind: {word})")
            }
        }
        PowerDeviceKind::Lever => {
            if device.on { "Lever — on".to_string() } else { "Lever — off".to_string() }
        }
        PowerDeviceKind::Button => {
            if device.on { "Button — pressed".to_string() } else { "Button — released".to_string() }
        }
        PowerDeviceKind::PressurePlate => {
            if device.on {
                "Pressure Plate — pressed".to_string()
            } else {
                "Pressure Plate — released".to_string()
            }
        }
        PowerDeviceKind::PlungerDetonator => {
            if device.on {
                "Plunger Detonator — pressed".to_string()
            } else {
                "Plunger Detonator — released".to_string()
            }
        }
        PowerDeviceKind::LogicGate => {
            let state = if device.on { "on" } else { "off" };
            format!("Logic Gate ({}) — output {state}", gate_op_name(device.gate_op))
        }
        PowerDeviceKind::ElectricLamp => {
            let lit = block == crate::block::ELECTRIC_LAMP_LIT;
            if lit { "Electric Lamp — lit".to_string() } else { "Electric Lamp — unlit".to_string() }
        }
        PowerDeviceKind::BeamSensor => {
            if device.on { "Beam Sensor — tripped".to_string() } else { "Beam Sensor — armed".to_string() }
        }
        PowerDeviceKind::MotionSensor => {
            if device.on {
                "Motion Sensor — tripped".to_string()
            } else {
                "Motion Sensor — armed".to_string()
            }
        }
        PowerDeviceKind::Mirror => "Mirror".to_string(),
        PowerDeviceKind::BlastingKeg => {
            if device.charge > 0 {
                format!("Blasting Keg — fuse lit! {} s", ceil_secs(device.charge))
            } else {
                "Blasting Keg — idle".to_string()
            }
        }
    }
}

/// Draw the power-device hover label for one player. Caller passes the
/// current `target_block` (from raycast), the device data, and `fed` (is
/// this cell currently driven by an adjacent source/cable — only Battery's
/// text uses it, for the "charging" word; the caller computes it with
/// `power::is_block_powered` since that needs the `World` this pure-render
/// module doesn't otherwise touch). Anchored the same way as
/// `campfire_ui::draw_campfire_hover_label` — viewport-relative, since egui
/// doesn't have the camera plumbing here to project a world-space point.
pub fn draw_power_hover_label(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    device: &PowerDeviceData,
    block: BlockId,
    fed: bool,
    windmill: WindmillView,
    registry: &BlockRegistry,
) {
    let text = power_hover_text(device, block, fed, windmill);
    let _ = registry; // reserved for future item-name display (unused today)

    let panel_origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - 150.0,
        viewport.y as f32 + viewport.height as f32 / 3.0,
    );
    egui::Area::new(egui::Id::new(("power_hover", player_index)))
        .fixed_pos(panel_origin)
        .interactable(false)
        .order(egui::Order::Tooltip)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(10, 14, 18, 220))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_min_width(280.0);
                        for (i, line) in text.split('\n').enumerate() {
                            let rich = if i == 0 {
                                egui::RichText::new(line)
                                    .size(16.0)
                                    .color(egui::Color32::from_rgb(120, 200, 255))
                                    .strong()
                            } else {
                                egui::RichText::new(line)
                                    .size(13.0)
                                    .color(egui::Color32::LIGHT_GRAY)
                            };
                            ui.label(rich);
                        }
                    });
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::Facing;

    fn dev(kind: PowerDeviceKind) -> PowerDeviceData {
        PowerDeviceData::new(kind, Facing::North)
    }

    #[test]
    fn steam_generator_burning_shows_seconds_from_fuel_ticks() {
        let mut d = dev(PowerDeviceKind::SteamGenerator);
        let mut fuel = crate::furnace::FurnaceData::default();
        fuel.lit = true;
        fuel.fuel_ticks_remaining = 41; // ceil(41/20) = 3
        d.fuel = Some(fuel);
        let text = power_hover_text(&d, crate::block::STEAM_GENERATOR_LIT, false, WindmillView::default());
        assert_eq!(text, "Steam Generator — burning, about 3 s left");
    }

    #[test]
    fn steam_generator_burning_shows_queued_fuel() {
        let mut d = dev(PowerDeviceKind::SteamGenerator);
        let mut fuel = crate::furnace::FurnaceData::default();
        fuel.lit = true;
        fuel.fuel_ticks_remaining = 20;
        fuel.fuel = Some(crate::item::ItemStack::new_material(crate::item::MaterialId::Coal, 5));
        d.fuel = Some(fuel);
        let text = power_hover_text(&d, crate::block::STEAM_GENERATOR_LIT, false, WindmillView::default());
        assert_eq!(text, "Steam Generator — burning, about 1 s left (+ 5 more)");
    }

    #[test]
    fn steam_generator_cold_prompts_for_fuel() {
        let d = dev(PowerDeviceKind::SteamGenerator);
        let text = power_hover_text(&d, crate::block::STEAM_GENERATOR, false, WindmillView::default());
        assert_eq!(text, "Steam Generator — cold. Right-click with fuel.");
    }

    #[test]
    fn battery_zero_charge_reads_empty() {
        let d = dev(PowerDeviceKind::Battery);
        let text = power_hover_text(&d, crate::block::BATTERY, false, WindmillView::default());
        assert_eq!(text, "Battery — empty");
    }

    #[test]
    fn battery_full_charge_reads_100_percent() {
        let mut d = dev(PowerDeviceKind::Battery);
        d.charge = BATTERY_CAPACITY;
        let text = power_hover_text(&d, crate::block::BATTERY, false, WindmillView::default());
        assert_eq!(text, "Battery — 100 % charge");
    }

    #[test]
    fn battery_being_fed_shows_charging() {
        let mut d = dev(PowerDeviceKind::Battery);
        d.charge = 0;
        let text = power_hover_text(&d, crate::block::BATTERY, true, WindmillView::default());
        assert_eq!(text, "Battery — empty, charging");
    }

    #[test]
    fn hand_crank_turning_shows_seconds_left() {
        let mut d = dev(PowerDeviceKind::HandCrank);
        d.charge = 21; // ceil(21/20) = 2
        let text = power_hover_text(&d, crate::block::HAND_CRANK, false, WindmillView::default());
        assert_eq!(text, "Hand Crank — turning, 2 s left");
    }

    #[test]
    fn hand_crank_still_prompts_to_crank() {
        let d = dev(PowerDeviceKind::HandCrank);
        let text = power_hover_text(&d, crate::block::HAND_CRANK, false, WindmillView::default());
        assert_eq!(text, "Hand Crank — still. Right-click to crank.");
    }

    /// Wind wave §2.2 — the Windmill's three states. `blocked` wins over the
    /// wind word: a walled-in mill is the failure a builder can act on.
    #[test]
    fn windmill_reads_turning_still_or_blocked() {
        let d = dev(PowerDeviceKind::Windmill);
        let open = |speed| WindmillView { speed, exposed: true };

        assert_eq!(
            power_hover_text(&d, crate::block::WINDMILL_TURNING, false, open(0.45)),
            "Windmill — turning (wind: fresh)"
        );
        assert_eq!(
            power_hover_text(&d, crate::block::WINDMILL, false, open(0.25)),
            "Windmill — still (wind: light)"
        );
        assert_eq!(
            power_hover_text(
                &d,
                crate::block::WINDMILL,
                false,
                WindmillView { speed: 0.90, exposed: false },
            ),
            "Windmill — blocked (needs open sky)",
            "no sky beats any amount of wind"
        );
    }

    /// Whole-branch review, minor. `WindmillView`'s default is passed by every
    /// non-Windmill call site as filler, so it must not be the accusing state:
    /// a derived `Default` gave `exposed: false` and would have told a builder
    /// their perfectly open mill was walled in the moment a caller forgot to
    /// compute exposure.
    #[test]
    fn windmill_view_default_is_not_the_blocked_reading() {
        assert!(WindmillView::default().exposed, "the benign reading is the default");
        let d = dev(PowerDeviceKind::Windmill);
        assert_eq!(
            power_hover_text(&d, crate::block::WINDMILL, false, WindmillView::default()),
            "Windmill — still (wind: calm)",
            "the default must never read as 'blocked (needs open sky)'"
        );
    }

    #[test]
    fn water_wheel_turning() {
        let d = dev(PowerDeviceKind::WaterWheel);
        let text = power_hover_text(&d, crate::block::WATER_WHEEL_TURNING, false, WindmillView::default());
        assert_eq!(text, "Water Wheel — turning");
    }

    #[test]
    fn water_wheel_still_needs_a_stream() {
        let d = dev(PowerDeviceKind::WaterWheel);
        let text = power_hover_text(&d, crate::block::WATER_WHEEL, false, WindmillView::default());
        assert_eq!(text, "Water Wheel — still. It needs a stream, not a pond.");
    }

    #[test]
    fn lever_on_off() {
        let mut d = dev(PowerDeviceKind::Lever);
        d.on = true;
        assert_eq!(power_hover_text(&d, crate::block::LEVER, false, WindmillView::default()), "Lever — on");
        d.on = false;
        assert_eq!(power_hover_text(&d, crate::block::LEVER, false, WindmillView::default()), "Lever — off");
    }

    #[test]
    fn button_pressed_released() {
        let mut d = dev(PowerDeviceKind::Button);
        d.on = true;
        assert_eq!(power_hover_text(&d, crate::block::BUTTON, false, WindmillView::default()), "Button — pressed");
    }

    #[test]
    fn pressure_plate_pressed_released() {
        let d = dev(PowerDeviceKind::PressurePlate);
        assert_eq!(
            power_hover_text(&d, crate::block::PRESSURE_PLATE, false, WindmillView::default()),
            "Pressure Plate — released"
        );
    }

    #[test]
    fn plunger_detonator_pressed() {
        let mut d = dev(PowerDeviceKind::PlungerDetonator);
        d.on = true;
        assert_eq!(
            power_hover_text(&d, crate::block::PLUNGER_DETONATOR, false, WindmillView::default()),
            "Plunger Detonator — pressed"
        );
    }

    #[test]
    fn logic_gate_shows_op_and_output() {
        let mut d = dev(PowerDeviceKind::LogicGate);
        d.gate_op = GateOp::Xor;
        d.on = true;
        assert_eq!(
            power_hover_text(&d, crate::block::LOGIC_GATE, false, WindmillView::default()),
            "Logic Gate (XOR) — output on"
        );
    }

    #[test]
    fn electric_lamp_lit_unlit() {
        let d = dev(PowerDeviceKind::ElectricLamp);
        assert_eq!(
            power_hover_text(&d, crate::block::ELECTRIC_LAMP_LIT, false, WindmillView::default()),
            "Electric Lamp — lit"
        );
        assert_eq!(
            power_hover_text(&d, crate::block::ELECTRIC_LAMP, false, WindmillView::default()),
            "Electric Lamp — unlit"
        );
    }

    #[test]
    fn beam_sensor_tripped_armed() {
        let mut d = dev(PowerDeviceKind::BeamSensor);
        d.on = true;
        assert_eq!(power_hover_text(&d, crate::block::BEAM_SENSOR, false, WindmillView::default()), "Beam Sensor — tripped");
        d.on = false;
        assert_eq!(power_hover_text(&d, crate::block::BEAM_SENSOR, false, WindmillView::default()), "Beam Sensor — armed");
    }

    #[test]
    fn motion_sensor_tripped_armed() {
        let mut d = dev(PowerDeviceKind::MotionSensor);
        d.on = true;
        assert_eq!(
            power_hover_text(&d, crate::block::MOTION_SENSOR, false, WindmillView::default()),
            "Motion Sensor — tripped"
        );
    }

    #[test]
    fn mirror_is_just_its_name() {
        let d = dev(PowerDeviceKind::Mirror);
        assert_eq!(power_hover_text(&d, crate::block::MIRROR, false, WindmillView::default()), "Mirror");
    }

    #[test]
    fn blasting_keg_fuse_lit_shows_seconds() {
        let mut d = dev(PowerDeviceKind::BlastingKeg);
        d.charge = crate::power::KEG_FUSE_TICKS; // 80 ticks -> 4s
        assert_eq!(
            power_hover_text(&d, crate::block::BLASTING_KEG, false, WindmillView::default()),
            "Blasting Keg — fuse lit! 4 s"
        );
    }

    #[test]
    fn blasting_keg_idle_when_unlit() {
        let d = dev(PowerDeviceKind::BlastingKeg);
        assert_eq!(power_hover_text(&d, crate::block::BLASTING_KEG, false, WindmillView::default()), "Blasting Keg — idle");
    }
}

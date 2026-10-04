//! egui integration — manages egui context, winit event handling, and wgpu rendering.
//!
//! Provides the bridge between winit input, egui UI building, and wgpu rendering.
//! Block textures from the atlas are uploaded as egui managed textures for inventory display.

use winit::window::Window;

/// Wraps egui state for the game.
pub struct EguiIntegration {
    pub ctx: egui::Context,
    /// None in headless mode (no window for event handling).
    state: Option<egui_winit::State>,
    renderer: egui_wgpu::Renderer,
    /// egui TextureIds for each block texture layer (indexed by layer).
    block_texture_ids: Vec<egui::TextureId>,
}

impl EguiIntegration {
    /// Create a new egui integration.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        window: &Window,
    ) -> Self {
        let ctx = egui::Context::default();

        // Configure default style: dark theme with game-appropriate colours
        let mut style = egui::Style { visuals: egui::Visuals::dark(), ..Default::default() };
        style.visuals.window_corner_radius = egui::CornerRadius::same(4);
        // Copperline (brand/BRAND-GUIDELINES.md): Deep Frontier panels, Deep Rock
        // buttons. Menu text colours are contrast-checked (>= 4.5:1) against these.
        style.visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgba_unmultiplied(18, 32, 36, 220);
        style.visuals.widgets.inactive.bg_fill = crate::brand::DEEP_ROCK;
        style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(52, 52, 50);
        style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(64, 62, 58);
        // Keyboard/controller focus uses the "active" widget visuals (egui
        // 0.34 style(): has_focus → active) — give it a visible gold stroke
        // so d-pad navigation reads on standard widgets (Lantern `#F4C16F`; the
        // crafting UI's PAD_FOCUS_BORDER is a separate, later pass). Buttons that override .stroke() paint their
        // own ring via menu::focus_ring.
        style.visuals.widgets.active.bg_stroke =
            egui::Stroke::new(2.0_f32, crate::brand::LANTERN);
        // Selection: Forest Green fill with Stone text (4.8:1).
        style.visuals.selection.bg_fill = crate::brand::FOREST;
        style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, crate::brand::STONE);

        // Android gets thumb-sized hit targets. egui's defaults are mouse-sized
        // (an interact height of ~18 points); an egui point is effectively a dp
        // here and Material's minimum touch target is 48dp, so stock widgets are
        // under half what a thumb needs — on a 420dpi phone ~4mm of target for
        // an 8mm finger, which reads as "the button didn't work".
        //
        // Android only, not every TOUCH_PLATFORM: the web build is one binary
        // for desktop browsers AND tablets, and the live web taster's layout is
        // not to change under its desktop users. Web tablets can opt in later
        // off the runtime `touch_input::is_touch_device()`.
        if cfg!(target_os = "android") {
            style.spacing.interact_size.y = 44.0;
            style.spacing.button_padding = egui::vec2(12.0, 10.0);
            style.spacing.item_spacing.y = 8.0;
            style.spacing.scroll.bar_width = 12.0;
            style.spacing.scroll.handle_min_length = 32.0;
            // egui 0.34 defaults scrollbars to FLOATING, where they appear only
            // once you are already scrolling — useless as a discovery cue (and
            // AlwaysVisible still drew nothing with floating on, verified on a
            // Pixel 8). Non-floating reserves real width and paints the bar
            // unconditionally: the 2026-05-21 playtest showed kids read a
            // clipped view as "that's all there is" rather than "scroll me".
            style.spacing.scroll.floating = false;
        }

        ctx.set_global_style(style);

        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            None, // native_pixels_per_point: auto-detect
            None, // max_texture_side
            None, // theme
        );

        let renderer = egui_wgpu::Renderer::new(
            device,
            surface_format,
            egui_wgpu::RendererOptions {
                msaa_samples: 1,
                depth_stencil_format: None, // egui renders without depth
                dithering: true,
                predictable_texture_filtering: false,
            },
        );

        Self {
            ctx,
            state: Some(state),
            renderer,
            block_texture_ids: Vec::new(),
        }
    }

    /// Create a headless egui integration (for screenshot mode — no window).
    /// egui won't be used for interactive rendering, but the struct must exist.
    pub fn new_headless(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        _textures: &[Vec<u8>],
    ) -> Self {
        let ctx = egui::Context::default();

        let renderer = egui_wgpu::Renderer::new(
            device,
            surface_format,
            egui_wgpu::RendererOptions {
                msaa_samples: 1,
                depth_stencil_format: None,
                dithering: true,
                predictable_texture_filtering: false,
            },
        );

        Self {
            ctx,
            state: None,
            renderer,
            block_texture_ids: Vec::new(),
        }
    }

    /// Upload block textures from the atlas as egui managed textures.
    /// Call once after atlas build and again when resource pack changes.
    ///
    /// `tex_size` is only a **fallback** for a malformed (non-square) buffer —
    /// each icon is otherwise sized from its own pixel data. The atlas
    /// resolution follows the active texture pack (16/32/64/128, Spec 03
    /// §11.3), and trusting a caller-supplied constant here (P6 audit finding)
    /// desynced `queue.write_texture`'s layout from the buffer it was copying,
    /// panicking on the very first non-16×16 pack.
    pub fn upload_block_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        textures: &[Vec<u8>],
        tex_size: u32,
    ) {
        // Free old textures
        for id in self.block_texture_ids.drain(..) {
            self.renderer.free_texture(&id);
        }

        for pixels in textures {
            let side = icon_upload_side(pixels.len(), tex_size);
            let texture_id = self.renderer.register_native_texture(
                device,
                &create_egui_texture_view(device, queue, pixels, side),
                wgpu::FilterMode::Nearest, // Preserve pixel art crispness
            );
            self.block_texture_ids.push(texture_id);
        }
    }

    /// Get the egui TextureId for a given block texture layer.
    pub fn block_texture(&self, layer: u32) -> Option<egui::TextureId> {
        self.block_texture_ids.get(layer as usize).copied()
    }

    /// Register an arbitrary wgpu texture view as an egui texture and return its
    /// id. Used by the #17 "Your look" 3D avatar preview, which renders the
    /// avatar into an offscreen target and shows it in the panel. The view is
    /// stable (its contents are re-rendered each frame), so callers register
    /// once and reuse the id.
    pub fn register_native_texture(
        &mut self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
        filter: wgpu::FilterMode,
    ) -> egui::TextureId {
        self.renderer.register_native_texture(device, view, filter)
    }

    /// Free a texture previously registered via `register_native_texture`.
    /// Used when a wardrobe entry (and its cached 3D thumbnail) is deleted.
    pub fn free_native_texture(&mut self, id: egui::TextureId) {
        self.renderer.free_texture(&id);
    }

    /// Pass a winit WindowEvent to egui. Returns true if egui consumed the event.
    pub fn on_window_event(&mut self, window: &Window, event: &winit::event::WindowEvent) -> bool {
        if let Some(state) = &mut self.state {
            let response = state.on_window_event(window, event);
            response.consumed
        } else {
            false
        }
    }

    /// Begin a new egui frame. Call once per frame before building UI.
    /// `window: None` (the headless GameState harness, 2026-07-11) yields a
    /// default `RawInput` — egui is pure CPU, so every UI arm's LOGIC still
    /// runs headless; only the surface paint is skipped (see
    /// [`Self::end_frame_discard`]).
    pub fn begin_frame(&mut self, window: Option<&Window>) -> egui::RawInput {
        if let (Some(state), Some(window)) = (&mut self.state, window) {
            state.take_egui_input(window)
        } else {
            egui::RawInput::default()
        }
    }

    /// Headless harness — balance a `begin_pass` when there is no surface to
    /// paint to: end the egui pass and discard the output. (The painted paths
    /// end the pass inside `Renderer::render` / `render_menu_only`.)
    pub fn end_frame_discard(&mut self) {
        let _ = self.ctx.end_pass();
    }

    /// End the egui pass: tessellate, upload textures, render.
    /// Call after all UI building is done (after begin_pass + UI code).
    pub fn end_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        screen_descriptor: egui_wgpu::ScreenDescriptor,
        window: &Window,
    ) {
        let full_output = self.ctx.end_pass();

        // Handle platform output (cursor icon, clipboard, etc.)
        if let Some(state) = &mut self.state {
            state.handle_platform_output(window, full_output.platform_output);
        }

        let paint_jobs = self.ctx.tessellate(full_output.shapes, full_output.pixels_per_point);

        // Upload texture deltas
        for (id, delta) in &full_output.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, delta);
        }

        // Upload vertex/index buffers (must happen before render)
        self.renderer.update_buffers(device, queue, encoder, &paint_jobs, &screen_descriptor);

        // Render egui
        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // Render on top of existing content
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            // forget_lifetime decouples the render pass from the encoder borrow,
            // trading compile-time safety for the 'static lifetime egui-wgpu requires.
            let mut render_pass = render_pass.forget_lifetime();

            self.renderer.render(&mut render_pass, &paint_jobs, &screen_descriptor);
        }

        // Free textures
        for id in &full_output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }

    /// Headless one-shot egui paint (no window / winit state). Builds a frame at
    /// the given logical size, runs `run_ui` to populate it, and paints the result
    /// into `view`. Used by the `--shot-lobby` screenshot mode to capture menu
    /// screens offscreen. `view` is expected to be pre-cleared by the caller.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn run_and_paint_headless(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        screen_descriptor: egui_wgpu::ScreenDescriptor,
        run_ui: impl FnOnce(&egui::Context),
    ) {
        let ppp = screen_descriptor.pixels_per_point;
        let size_pts = egui::vec2(
            screen_descriptor.size_in_pixels[0] as f32 / ppp,
            screen_descriptor.size_in_pixels[1] as f32 / ppp,
        );
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), size_pts)),
            ..Default::default()
        };
        self.ctx.begin_pass(raw_input);
        run_ui(&self.ctx);
        let full_output = self.ctx.end_pass();
        let paint_jobs = self.ctx.tessellate(full_output.shapes, full_output.pixels_per_point);
        for (id, delta) in &full_output.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, delta);
        }
        self.renderer.update_buffers(device, queue, encoder, &paint_jobs, &screen_descriptor);
        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui_headless_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            let mut render_pass = render_pass.forget_lifetime();
            self.renderer.render(&mut render_pass, &paint_jobs, &screen_descriptor);
        }
        for id in &full_output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }

    /// Check if egui wants exclusive keyboard input (e.g., text field focused).
    /// Task 16 — consulted each tick in `GameState::tick`'s per-player input
    /// gate (folded into `gameplay_input_suppressed`) so typing in a text
    /// field (world-name box, item search, sign edit, …) can't drive WASD.
    pub fn wants_keyboard_input(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }

    /// Check if egui wants exclusive pointer/mouse input. Same caller as
    /// `wants_keyboard_input` above — gates break/place while a click or
    /// drag is aimed at an egui widget rather than the world.
    pub fn wants_pointer_input(&self) -> bool {
        self.ctx.egui_wants_pointer_input()
    }
}

/// The square side (in pixels) to upload one egui block-icon at, given its
/// RGBA buffer's byte length. Pure so it's unit-testable without a GPU
/// device/queue — the panic-causing part of the P6 "saved texture pack above
/// 16×16 panics on every launch" finding was exactly this size computation
/// (a hard-coded `16` fed to `write_texture` regardless of the buffer's real
/// resolution). `fallback` is used only when `pixels_len` isn't a whole
/// square RGBA buffer (0 bytes, or corrupt data) — logged as a warning by the
/// caller, never asserted/panicked on.
fn icon_upload_side(pixels_len: usize, fallback: u32) -> u32 {
    let side = crate::texture_registry::square_side(pixels_len);
    if side == 0 {
        log::warn!(
            "egui icon upload: texture buffer of {pixels_len} bytes isn't a square RGBA \
             image — falling back to {fallback}x{fallback}"
        );
        fallback
    } else {
        side
    }
}

/// Create a wgpu TextureView from RGBA pixel data for use as an egui native texture.
fn create_egui_texture_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pixels: &[u8],
    size: u32,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("egui_block_texture"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size * 4),
            rows_per_image: Some(size),
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );

    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_upload_side_reads_32x32_pack_correctly() {
        // A 32×32 RGBA layer — the P6 crash-loop scenario ("player picks a
        // 32×32 pack in Settings"). Must size from the buffer, not the
        // hard-coded 16 that used to be passed at the call site.
        let pixels = vec![0u8; 32 * 32 * 4];
        assert_eq!(icon_upload_side(pixels.len(), 16), 32);
    }

    #[test]
    fn icon_upload_side_reads_16x16_pack_correctly() {
        let pixels = vec![0u8; 16 * 16 * 4];
        assert_eq!(icon_upload_side(pixels.len(), 16), 16);
    }

    #[test]
    fn icon_upload_side_reads_64_and_128_packs_correctly() {
        assert_eq!(icon_upload_side(64 * 64 * 4, 16), 64);
        assert_eq!(icon_upload_side(128 * 128 * 4, 16), 128);
    }

    #[test]
    fn icon_upload_side_falls_back_on_malformed_buffer_without_panicking() {
        // Empty buffer (e.g. a decode failure upstream) must never panic —
        // it falls back to the caller-supplied default.
        assert_eq!(icon_upload_side(0, 16), 16);
        // Anything under one pixel's worth of bytes (< 4) is the same
        // "nothing usable here" case — `square_side` floors to 0 and the
        // fallback kicks in, rather than sizing a zero-dimension texture.
        assert_eq!(icon_upload_side(3, 16), 16);
    }

    #[test]
    fn icon_upload_side_sizes_a_slightly_off_buffer_to_its_nearest_square_side() {
        // A byte length that ISN'T an exact side×side×4 (71 bytes, not a
        // multiple of 4 at all) still yields a non-zero side from
        // `square_side`'s floor(sqrt(..)) — `icon_upload_side` only falls
        // back to the caller's default when that floor is 0 (< 1 pixel of
        // data), not merely "not a perfect square". This documents that
        // boundary rather than asserting a fallback that never fires here.
        assert_eq!(icon_upload_side(4 * 4 * 4 + 7, 16), 4);
    }
}

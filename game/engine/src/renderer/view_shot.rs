//! A whole-frame picture of one player's live view (world, exhibits,
//! particles, through the frame's own `render_world_viewport`) with an egui
//! overlay on top, written to a PNG. For the headless harness only: the
//! Trials island's screenshots are taken through it.

use super::{create_crosshair_buffer, render_world_viewport, EguiIntegration, Renderer};

impl Renderer {
    /// Draw player `player_index`'s view at the headless target's size, then
    /// `run_ui` over it, and save the result to `path`. Needs a headless renderer.
    pub fn render_view_to_png(
        &mut self,
        path: &str,
        player_index: usize,
        run_ui: impl FnOnce(&egui::Context, &EguiIntegration),
    ) {
        let Some(target) = self.color_target.as_ref() else { return };
        let Some(gpu) = self.player_gpu.get(player_index) else { return };
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let (width, height) = (self.width, self.height);
        let crosshair = create_crosshair_buffer(&self.device, width, height);
        let frustum = crate::camera::Frustum::from_view_proj(gpu.last_view_proj);
        let vp = crate::screen::ViewportRect { x: 0, y: 0, width, height };
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("view_shot_encoder"),
        });
        let _ = render_world_viewport(
            &self.device,
            &mut encoder,
            &view,
            &self.depth_texture_view,
            gpu,
            &vp,
            self.sky_color,
            true,
            &self.sky_pipeline,
            &self.render_pipeline,
            &self.water_pipeline,
            &self.transparent_pipeline,
            &self.transparent_depth_pipeline,
            &self.entity_pipeline,
            &self.avatar_pipeline,
            &self.wire_pipeline,
            &self.crack_pipeline,
            &self.crosshair_pipeline,
            &self.texture_bind_group,
            &self.skin_bind_group,
            &self.chunk_meshes,
            &self.water_meshes,
            &self.transparent_meshes,
            &self.cutout_meshes,
            &self.plant_pipeline,
            &self.plant_geo_vbuf,
            &self.plant_geo_ibuf,
            self.plant_geo_index_count,
            &self.plant_meshes,
            &self.particle_pipeline,
            &self.particle_geo_vbuf,
            &self.particle_geo_ibuf,
            self.particle_geo_index_count,
            &self.particle_instance_buffer,
            self.particle_count,
            &self.decal_pipeline,
            &self.decal_meshes,
            &self.micro_pipeline,
            &self.micro_geo,
            &self.micro_meshes,
            &self.micro_billboard_meshes,
            self.micro_lod_dist,
            self.chunk_draw_columns,
            &self.painting_pipeline,
            &self.paintings,
            &self.painting_textures,
            &self.painting_alpha_pipeline,
            &self.billboards,
            &frustum,
            &crosshair,
            false,
        );
        self.queue.submit(std::iter::once(encoder.finish()));
        // The UI pass loads the world drawn above and paints over it.
        // No fade-in: a one-off frame would catch a new panel half-drawn.
        let animation_time = self.egui.ctx.global_style().animation_time;
        self.egui.ctx.global_style_mut(|style| style.animation_time = 0.0);
        let mut run_ui = Some(run_ui);
        self.render_headless_ui_to_png(path, false, |ctx, assets| {
            if let Some(f) = run_ui.take() {
                f(ctx, assets)
            }
        });
        self.egui.ctx.global_style_mut(|style| style.animation_time = animation_time);
    }
}

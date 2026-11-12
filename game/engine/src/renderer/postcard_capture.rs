//! Trial postcards: draw one player's world view (no HUD, no egui, no
//! crosshair) into a small off-screen picture and read it back without ever
//! blocking. The world is drawn a second time through the same
//! `render_world_viewport` the frame uses, into a target sized to the player's
//! viewport aspect, so the picture is neither stretched nor covered by the
//! completion card. Native polls the device each frame; on the web the map
//! callback lands on its own. Any failure is silent: the caller just gets no
//! postcard. The flow around it is `postcard_flow.rs`.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use super::{create_crosshair_buffer, create_depth_texture, render_world_viewport, Renderer};

/// Width the world is drawn at for a postcard (twice the stored width, so the
/// reduction averages 2×2 and smooths the 16-px textures).
const CAPTURE_W: u32 = 640;

const PENDING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

/// A capture in flight.
pub struct PostcardCapture {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    bytes_per_row: u32,
    bgra: bool,
    state: Arc<AtomicU8>,
}

/// What a poll found.
pub enum PostcardPoll {
    /// Nothing asked for, or nothing in flight.
    Idle,
    /// Still on the GPU.
    Pending,
    /// The frame, RGBA, `width`×`height`.
    Ready { rgba: Vec<u8>, width: u32, height: u32 },
    /// It could not be read back.
    Failed,
}

/// The capture size for a viewport aspect (width / height), even and bounded.
pub fn capture_size(aspect: f32) -> (u32, u32) {
    let aspect = if aspect.is_finite() && aspect > 0.1 { aspect.min(10.0) } else { 16.0 / 9.0 };
    let h = ((CAPTURE_W as f32 / aspect).round() as u32).clamp(64, 2048);
    (CAPTURE_W, h)
}

impl Renderer {
    /// Start a postcard of player `player_index`'s view. `aspect` is that
    /// player's viewport width / height (the camera's projection is built for
    /// it). Returns false if one is already in flight, the player has no GPU
    /// state, or the surface format can't be read back.
    pub fn start_postcard_capture(&mut self, player_index: usize, aspect: f32) -> bool {
        if self.postcard_capture.is_some() {
            return false;
        }
        let bgra = match self.format {
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => true,
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => false,
            _ => return false,
        };
        let Some(gpu) = self.player_gpu.get(player_index) else { return false };
        let (width, height) = capture_size(aspect);
        let color = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("postcard_color"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = create_depth_texture(&self.device, width, height);
        let crosshair = create_crosshair_buffer(&self.device, width, height);
        let frustum = crate::camera::Frustum::from_view_proj(gpu.last_view_proj);
        let vp = crate::screen::ViewportRect { x: 0, y: 0, width, height };
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("postcard_encoder"),
        });
        let _ = render_world_viewport(
            &self.device,
            &mut encoder,
            &color_view,
            &depth_view,
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
        let bytes_per_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("postcard_staging"),
            size: (bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit(std::iter::once(encoder.finish()));
        let state = Arc::new(AtomicU8::new(PENDING));
        let flag = state.clone();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            flag.store(if result.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
        });
        self.postcard_capture = Some(PostcardCapture { buffer, width, height, bytes_per_row, bgra, state });
        true
    }

    /// Check the capture in flight. Never blocks.
    pub fn poll_postcard_capture(&mut self) -> PostcardPoll {
        let Some(cap) = self.postcard_capture.as_ref() else { return PostcardPoll::Idle };
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.device.poll(wgpu::PollType::Poll);
        match cap.state.load(Ordering::Acquire) {
            PENDING => PostcardPoll::Pending,
            MAPPED => {
                let cap = self.postcard_capture.take().expect("checked above");
                let rgba = {
                    let data = cap.buffer.slice(..).get_mapped_range();
                    unpad_rows(&data, cap.width, cap.height, cap.bytes_per_row, cap.bgra)
                };
                cap.buffer.unmap();
                PostcardPoll::Ready { rgba, width: cap.width, height: cap.height }
            }
            _ => {
                self.postcard_capture = None;
                PostcardPoll::Failed
            }
        }
    }

    /// Drop any capture in flight (a world change, a retry).
    pub fn cancel_postcard_capture(&mut self) {
        self.postcard_capture = None;
    }
}

/// Strip the row padding a buffer copy needs, swapping BGRA to RGBA if asked.
pub fn unpad_rows(data: &[u8], width: u32, height: u32, bytes_per_row: u32, bgra: bool) -> Vec<u8> {
    let row = (width * 4) as usize;
    let mut out = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        let start = y * bytes_per_row as usize;
        let Some(src) = data.get(start..start + row) else { break };
        if bgra {
            for p in src.chunks_exact(4) {
                out.extend_from_slice(&[p[2], p[1], p[0], 255]);
            }
        } else {
            for p in src.chunks_exact(4) {
                out.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_size_follows_the_viewport_aspect() {
        assert_eq!(capture_size(16.0 / 9.0), (640, 360));
        assert_eq!(capture_size(1.6), (640, 400));
        assert_eq!(capture_size(f32::NAN), (640, 360));
        assert_eq!(capture_size(100.0).1, 64);
    }

    #[test]
    fn rows_lose_their_padding_and_bgra_becomes_rgba() {
        // 2×2, padded rows of 12 bytes.
        let data = [
            1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0, //
            9, 10, 11, 12, 13, 14, 15, 16, 0, 0, 0, 0,
        ];
        assert_eq!(
            unpad_rows(&data, 2, 2, 12, false),
            vec![1, 2, 3, 255, 5, 6, 7, 255, 9, 10, 11, 255, 13, 14, 15, 255]
        );
        assert_eq!(unpad_rows(&data, 2, 1, 12, true), vec![3, 2, 1, 255, 7, 6, 5, 255]);
    }
}

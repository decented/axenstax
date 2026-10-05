//! wgpu renderer — sets up the GPU pipeline and draws textured chunk meshes,
//! crosshair overlay, hotbar HUD, and block highlight wireframe.

use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::camera::CameraUniform;
use crate::mesh::{ChunkMesh, Vertex};
use crate::texture_gen;
use crate::egui_integration::EguiIntegration;

/// Per-chunk GPU buffers.
pub struct GpuChunkMesh {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
}

/// #131 — the transparent-solid render pipeline (glass + future see-through
/// blocks). Identical to the water pipeline (alpha blend, depth read-only, no
/// back-face cull) but binds `fs_transparent`, which outputs the texture's own
/// alpha instead of water's blue tint. Built once and called from both the
/// native and headless renderer constructors so the two never drift.
fn build_transparent_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    pipeline_layout: &wgpu::PipelineLayout,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("transparent_render_pipeline"),
        layout: Some(pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Vertex::layout()],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_transparent"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// A chunk's instanced plants: one buffer of `PlantInstance` records drawn
/// against the shared unit cross-billboard geometry.
pub struct GpuPlantInstances {
    pub buffer: wgpu::Buffer,
    pub count: u32,
}

/// Owner-inbox #1/2/3 — a chunk's wallpaper face-overlay decals: one
/// non-indexed vertex buffer of `mesh::Vertex` triangles (6 verts per painted
/// face). Drawn in the decal pass with the alpha-blended, depth-biased overlay
/// pipeline. Per-chunk + dirty-gated, like `GpuPlantInstances`.
pub struct GpuDecalMesh {
    pub vertex_buffer: wgpu::Buffer,
    pub vertex_count: u32,
}

/// Build the instanced-plant pipeline (shared by the windowed and headless
/// renderers). Vertex buffer 0 = the unit cross geometry, buffer 1 = the
/// per-instance data; `cull_mode: None` so each quad shows from both sides.
fn create_plant_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("plant_instanced_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_plant"),
            compilation_options: Default::default(),
            buffers: &[
                crate::mesh::PlantGeoVertex::layout(),
                crate::mesh::PlantInstance::layout(),
            ],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_plant"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Particle billboard pipeline (2026-07-05): alpha-blended, depth READ-ONLY
/// (occluded by terrain/water, never occludes), cull off. Same bind layout as
/// the chunk pipeline (camera + texture array); expansion happens in
/// `vs_particle` from `CameraUniform.cam_right/cam_up`.
fn create_particle_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("particle_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_particle"),
            compilation_options: Default::default(),
            buffers: &[
                crate::mesh::PlantGeoVertex::layout(),
                crate::mesh::ParticleInstance::layout(),
            ],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_particle"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Shared particle GPU objects for a constructor: pipeline + unit-quad
/// geometry + the persistent instance buffer.
fn create_particle_gpu(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::Buffer, wgpu::Buffer, u32, wgpu::Buffer) {
    let pipeline = create_particle_pipeline(device, layout, shader, format);
    let (pq_verts, pq_indices) = crate::mesh::particle_unit_quad();
    let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("particle_geo_vbuf"),
        contents: bytemuck::cast_slice(&pq_verts),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("particle_geo_ibuf"),
        contents: bytemuck::cast_slice(&pq_indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    let inst = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("particle_instances"),
        size: (crate::particles::CAP_FULL * std::mem::size_of::<crate::mesh::ParticleInstance>())
            as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    (pipeline, vbuf, ibuf, pq_indices.len() as u32, inst)
}

/// Create the shared unit cross-billboard vertex + index buffers. Returns
/// `(vertex_buffer, index_buffer, index_count)`.
fn create_plant_geo(device: &wgpu::Device) -> (wgpu::Buffer, wgpu::Buffer, u32) {
    use wgpu::util::DeviceExt;
    let (verts, indices) = crate::mesh::plant_unit_cross();
    let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("plant_geo_vbuf"),
        contents: bytemuck::cast_slice(&verts),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("plant_geo_ibuf"),
        contents: bytemuck::cast_slice(&indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    (vbuf, ibuf, indices.len() as u32)
}

/// Owner-inbox #18 — the instanced micro-model pipeline. Vertex buffer 0 = a
/// baked shell's `Vertex` geometry (one shared mesh per registered type),
/// buffer 1 = the per-instance `MicroInstance` data. Reuses `fs_main` (opaque,
/// lit, no alpha-cutout — shells are solid blocks of texture); a bespoke
/// `vs_micro` applies the per-instance transform while keeping the shell's REAL
/// per-vertex normals (unlike `vs_plant`, which forces an up-normal for flat
/// billboards). `cull_mode: None` matches the plant path so there's no risk of
/// invisible faces at the Phase-C visual gate; Back-culling is a safe future
/// optimisation (shell winding is verified CCW-outward).
fn create_micro_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("micro_model_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_micro"),
            compilation_options: Default::default(),
            buffers: &[
                crate::mesh::Vertex::layout(),
                crate::mesh::MicroInstance::layout(),
            ],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Vertex for crosshair / hotbar HUD (screen-space 2D).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CrosshairVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

/// Vertex for wireframe overlay (world-space 3D).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WireVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

/// Per-player GPU resources. Each player gets their own camera uniform buffer
/// and bind group so viewports can render with independent cameras.
pub struct PlayerGpuResources {
    pub camera_buffer: wgpu::Buffer,
    pub camera_bind_group: wgpu::BindGroup,
    pub entity_buffer: Option<wgpu::Buffer>,
    pub entity_vertex_count: u32,
    /// P1-T4 — remote-player avatar skin geometry (box-unwrapped onto the 64x64
    /// skin atlas). Drawn with `avatar_pipeline` + `skin_bind_group` in its own
    /// pass right after the entity pass. Separate from `entity_buffer` because
    /// it samples the dedicated skin texture, not the block array.
    pub avatar_buffer: Option<wgpu::Buffer>,
    pub avatar_vertex_count: u32,
    pub wire_buffer: Option<wgpu::Buffer>,
    pub wire_vertex_count: u32,
    /// Spec 05 §2.2 — crack overlay for the block this player is currently
    /// mining. A textured cube hugging the block surface; `None` when the
    /// player isn't mining (or progress is 0).
    pub crack_buffer: Option<wgpu::Buffer>,
    pub crack_vertex_count: u32,
    /// Spec 24 Phase 8 — ghost-placement wireframe. Separate buffer
    /// from `wire_buffer` so block-targeting outlines and plan ghosts
    /// can coexist (player can sweep the cursor while a ghost is up
    /// and still see the targeting cube under the crosshair).
    pub ghost_buffer: Option<wgpu::Buffer>,
    pub ghost_vertex_count: u32,
    /// #8 — spawn-proof overlay: red wireframe cubes around surface cells where
    /// a mob can spawn. Its own buffer (like `ghost_buffer`) so it coexists with
    /// ghost placement + the block-targeting outline.
    pub spawn_marker_buffer: Option<wgpu::Buffer>,
    pub spawn_marker_vertex_count: u32,
    /// #9 — blueprint build-guide ghosts: per-cell wireframe cubes coloured by
    /// status (green=correct, white=missing, red=wrong). Its own buffer so it
    /// coexists with ghost placement + the spawn overlay.
    pub build_guide_buffer: Option<wgpu::Buffer>,
    pub build_guide_vertex_count: u32,
    /// Trials (⚡ Race) — the chase-ghost: a translucent wireframe runner replayed
    /// at the personal-best run's recorded transform. Its own buffer so it
    /// coexists with the other wire overlays.
    pub trial_ghost_buffer: Option<wgpu::Buffer>,
    pub trial_ghost_vertex_count: u32,
    /// Spec 48 Phase 2 — visible IR beams (Beam Sensor). A LineList polyline
    /// through each armed/active beam path. Its own buffer like the others.
    pub beam_buffer: Option<wgpu::Buffer>,
    pub beam_vertex_count: u32,
    /// Skin-painter precision aids (2026-09-06) — the texel GRID on the blown-up
    /// Workshop mannequin plus the hover FOOTPRINT box showing where the next
    /// click lands. Geometry comes from `skin_grid`; both sets share one buffer
    /// (they are rebuilt together every frame the painter is open) but keep
    /// their own colour + line thickness. Its own buffer like the others, so a
    /// plan ghost or the Bellows cage can be up at the same time.
    pub paint_aid_buffer: Option<wgpu::Buffer>,
    pub paint_aid_vertex_count: u32,
    /// Phase 7 — first-person viewmodel (local player's own arm + held item).
    /// Geometry is pre-transformed to VIEW space on the CPU, so it renders with
    /// an identity view matrix + a narrow-FOV projection held in its own
    /// camera uniform (separate from the world camera so the world's yaw/pitch
    /// never move the viewmodel).
    pub viewmodel_buffer: Option<wgpu::Buffer>,
    pub viewmodel_vertex_count: u32,
    /// P1-T5 — the first-person ARM, box-unwrapped onto the 64x64 skin atlas.
    /// Separate from `viewmodel_buffer` (the held item) because it draws with
    /// `avatar_pipeline` + `skin_bind_group` (the dedicated skin texture) while
    /// the held item draws with `entity_pipeline` + the block array. Both share
    /// the viewmodel camera and the one depth-cleared viewmodel pass.
    pub viewmodel_skin_buffer: Option<wgpu::Buffer>,
    pub viewmodel_skin_vertex_count: u32,
    pub viewmodel_camera_buffer: wgpu::Buffer,
    pub viewmodel_camera_bind_group: wgpu::BindGroup,
    /// CPU-side copy of this player's most recent world view-projection matrix,
    /// cached by `update_camera`. Used to build the culling frustum in `render`
    /// (Spec 39 A1) — the GPU camera buffer can't be read back cheaply.
    pub last_view_proj: glam::Mat4,
}

/// Per-frame draw/culling counters, surfaced in the F3 overlay (Spec 39 A1).
/// Accumulated across all viewports in a frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct DrawStats {
    /// Chunk/water/plant meshes actually submitted this frame.
    pub draw_calls: u32,
    /// Chunk/water/plant meshes skipped by frustum culling this frame.
    pub culled: u32,
}

/// Exhibits / paintings — a hi-res texture (its own native-resolution image) plus the
/// group-2 bind group `painting_pipeline` samples it through. ONE per unique
/// artwork/plaque image; many quads reference it by id, so a maze densely hung
/// from the same catalogue uploads each image only once.
struct PaintingTex {
    bind_group: wgpu::BindGroup,
    /// Kept alive so the bind group's view stays valid.
    _texture: wgpu::Texture,
}

/// Exhibits / paintings — one hung quad (a painting or a plaque). References its image by
/// `tex_id` (resolved against `painting_textures` at draw time) so the texture is
/// shared, not duplicated. Carries a world AABB for per-quad frustum culling.
struct PaintingQuad {
    tex_id: u64,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    aabb_min: glam::Vec3,
    aabb_max: glam::Vec3,
}

/// Exhibits — a standing **billboard** quad: a painting whose
/// facing is recomputed toward the viewer every frame (Y-axis only). Keeps its
/// anchor + size so `reorient_billboards` can rewrite its corners in place.
struct BillboardQuad {
    tex_id: u64,
    /// World-space centre of the quad (the standing exhibit's mount point).
    anchor: glam::Vec3,
    /// Quad height in blocks; width = height * aspect.
    height: f32,
    aspect: f32,
    /// Rewritten each frame (COPY_DST) — never reallocated.
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    aabb_min: glam::Vec3,
    aabb_max: glam::Vec3,
}

/// Build the four world-space corner vertices for a painting/billboard quad
/// facing `normal`, centred at `center`, `height` tall, `aspect` wide-per-tall.
/// Shared by the static wall path and the per-frame billboard rewrite so both
/// agree on winding + UVs (UV origin top-left; corners bl, br, tr, tl).
fn painting_quad_verts(
    center: glam::Vec3,
    normal: glam::Vec3,
    height: f32,
    aspect: f32,
) -> ([crate::mesh::Vertex; 4], glam::Vec3, glam::Vec3) {
    let hh = height * 0.5;
    let hw = height * aspect * 0.5;
    let normal = normal.normalize_or_zero();
    let up = glam::Vec3::Y;
    let right = up.cross(normal).normalize_or_zero();
    let bl = center - right * hw - up * hh;
    let br = center + right * hw - up * hh;
    let tr = center + right * hw + up * hh;
    let tl = center - right * hw + up * hh;
    let n = [normal.x, normal.y, normal.z];
    let mk = |p: glam::Vec3, uv: [f32; 2]| crate::mesh::Vertex {
        position: [p.x, p.y, p.z],
        normal: n,
        tex_layer: 0,
        uv,
        light: 1.0,
        sky_light: 0.0,
    };
    let verts = [
        mk(bl, [0.0, 1.0]),
        mk(br, [1.0, 1.0]),
        mk(tr, [1.0, 0.0]),
        mk(tl, [0.0, 0.0]),
    ];
    let aabb_min = bl.min(br).min(tr).min(tl);
    let aabb_max = bl.max(br).max(tr).max(tl);
    (verts, aabb_min, aabb_max)
}

/// Pick the surface **configuration** format and the format we actually **render**
/// through. Native backends expose an sRGB surface format directly, so we use it
/// for both. WebGPU only exposes non-sRGB canvas formats (`bgra8unorm` /
/// `rgba8unorm`): configuring with one and writing our linear shader output to it
/// leaves the scene un-gamma-encoded, so it renders far too dark — the WASM-only
/// "it is too dark" report. The fix is to keep the non-sRGB format for the canvas
/// config but render through an sRGB **view alias** declared in `view_formats`;
/// the GPU then gamma-encodes on write exactly like native does.
///
/// Returns `(config_format, render_format, view_formats)`. When the platform
/// already offers an sRGB format the render format equals the config format and
/// `view_formats` is empty — native behaviour is byte-for-byte unchanged.
fn choose_surface_formats(
    available: &[wgpu::TextureFormat],
) -> (wgpu::TextureFormat, wgpu::TextureFormat, Vec<wgpu::TextureFormat>) {
    let config_format = available
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(available[0]);
    let render_format = config_format.add_srgb_suffix();
    let view_formats = if render_format != config_format {
        vec![render_format]
    } else {
        vec![]
    };
    (config_format, render_format, view_formats)
}

pub struct Renderer {
    pub surface: Option<wgpu::Surface<'static>>,
    /// Retained so the surface can be REBUILT, not just dropped.
    ///
    /// Android destroys the native window every time the activity is
    /// backgrounded, then hands back a fresh one. Rebuilding a surface needs the
    /// instance it came from, and nothing else here holds one, so without this a
    /// dropped surface would be permanent and the app would sit on a blank
    /// screen. Desktop and web never exercise this path, hence dead there, but it
    /// stays compiled on every platform so it cannot rot behind a cfg.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    instance: wgpu::Instance,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// Format we render through (always sRGB so linear shader output is gamma-
    /// encoded on write). On native this is the surface's own format; on WebGPU
    /// it is an sRGB *view alias* of the non-sRGB canvas format (see
    /// [`choose_surface_formats`]).
    pub format: wgpu::TextureFormat,
    /// Format the surface/canvas is actually *configured* with. Equals `format`
    /// on native; the non-sRGB canvas format on WebGPU. Used when (re)configuring
    /// the surface (resize, present-mode change).
    config_format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
    /// Current surface present mode (Spec 39 — the vsync/frame-cap dial). Used by
    /// `resize` when reconfiguring; changed live via `set_present_mode`.
    pub present_mode: wgpu::PresentMode,
    pub render_pipeline: wgpu::RenderPipeline,
    pub depth_texture_view: wgpu::TextureView,
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
    pub texture_bind_group: wgpu::BindGroup,
    /// Spec 40 (The Workshop) — kept so the block texture array can be rebuilt at
    /// runtime when authored overrides change (append override layers, recreate
    /// the view + `texture_bind_group`). See `rebuild_block_textures`.
    block_sampler: wgpu::Sampler,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    /// The block texture array itself, retained so animated-texture layers can be
    /// re-uploaded per frame (Spec 03 §3.4) without a full atlas rebuild.
    block_texture: wgpu::Texture,
    /// Atlas resolution (square side) of `block_texture`, for per-layer uploads.
    block_tex_size: u32,
    /// Spec 39 A6 — whether `block_texture` carries a mip chain and
    /// `block_sampler` filters with it. Driven by the Graphics "Mipmaps" dial
    /// via [`Renderer::set_mipmaps`]; false (today's look) until turned on.
    block_mipmaps: bool,
    /// Animated textures from the active pack (Spec 03 §3.4). Each owns its frames
    /// + game-tick schedule; [`Renderer::advance_animated_textures`] uploads the
    ///   current frame whenever it changes. Empty (no cost) for the default pack.
    animated_textures: Vec<crate::texture_anim::AnimatedTexture>,
    /// Last frame index uploaded per animated texture (parallel to
    /// `animated_textures`); `u32::MAX` forces the first upload.
    animated_last_frame: Vec<u32>,
    /// Game-tick clock driving animation — set once per frame from the world
    /// `tick_counter`, so animation pauses exactly when the game does.
    anim_clock: u64,
    pub chunk_meshes: ahash::AHashMap<(i32, i32, i32), GpuChunkMesh>,
    /// Per-frame draw/cull counters (Spec 39 A1) — read by the F3 overlay.
    pub last_draw_stats: DrawStats,
    /// Crosshair vertex buffers keyed by viewport `(width, height)` (Spec 39 A2).
    /// The crosshair geometry is pixel-sized, so it depends on the viewport
    /// dimensions — but those only change on resize/layout, so caching here
    /// kills the per-frame `create_buffer_init` that ran for every viewport.
    crosshair_cache: ahash::AHashMap<(u32, u32), wgpu::Buffer>,
    /// Render scale (Spec 39 Phase 5). 1.0 = render the world straight to the
    /// surface (no cost). < 1.0 = render the world to `offscreen_*` at a smaller
    /// size then upscale-blit to the surface — the biggest weak-GPU lever (UI
    /// still draws crisp at native res on top).
    pub render_scale: f32,
    /// Offscreen world target (color view, depth view) + the blit bind group,
    /// present only when `render_scale < 1.0`.
    offscreen_color_view: Option<wgpu::TextureView>,
    offscreen_depth_view: Option<wgpu::TextureView>,
    offscreen_bind_group: Option<wgpu::BindGroup>,
    blit_pipeline: wgpu::RenderPipeline,
    blit_sampler: wgpu::Sampler,
    blit_bind_group_layout: wgpu::BindGroupLayout,
    // Water (transparent pass)
    pub water_pipeline: wgpu::RenderPipeline,
    pub water_meshes: ahash::AHashMap<(i32, i32, i32), GpuChunkMesh>,
    /// #131 — solid + transparent blocks (glass, …). Same alpha-blended, depth-
    /// read-only pipeline as water but `fs_transparent` (texture's own alpha, no
    /// blue tint). Per-chunk GPU meshes, drawn in the transparent pass.
    pub transparent_pipeline: wgpu::RenderPipeline,
    pub transparent_meshes: ahash::AHashMap<(i32, i32, i32), GpuChunkMesh>,
    // Instanced plants (flowers/grass/crops) — cross billboards
    pub plant_pipeline: wgpu::RenderPipeline,
    plant_geo_vbuf: wgpu::Buffer,
    plant_geo_ibuf: wgpu::Buffer,
    plant_geo_index_count: u32,
    pub plant_meshes: ahash::AHashMap<(i32, i32, i32), GpuPlantInstances>,
    // Particle framework (2026-07-05): shared unit quad + ONE persistent
    // instance buffer rewritten per frame (write_buffer prefix, draw 0..count).
    pub particle_pipeline: wgpu::RenderPipeline,
    particle_geo_vbuf: wgpu::Buffer,
    particle_geo_ibuf: wgpu::Buffer,
    particle_geo_index_count: u32,
    particle_instance_buffer: wgpu::Buffer,
    particle_count: u32,
    // Wallpaper face-overlay decals (owner-inbox #1/2/3) — per-chunk, like plants.
    pub decal_meshes: ahash::AHashMap<(i32, i32, i32), GpuDecalMesh>,
    // Owner-inbox #18 — micro-models. `micro_pipeline` draws a baked sub-voxel
    // shell per registered block type. `micro_geo` holds ONE shared baked
    // geometry per type (uploaded once from World::micro_registry via
    // `sync_micro_models`). `micro_meshes` holds the per-chunk, per-type instance
    // buffers — split by type because each type draws against its own shell
    // (unlike plants, which share one cross). Per-chunk + dirty-gated.
    pub micro_pipeline: wgpu::RenderPipeline,
    micro_geo: ahash::AHashMap<crate::block::BlockId, GpuChunkMesh>,
    pub micro_meshes:
        ahash::AHashMap<(i32, i32, i32), Vec<(crate::block::BlockId, GpuPlantInstances)>>,
    // Phase D far-LOD fallback billboards (one PlantInstance buffer per chunk, like
    // plant_meshes) — drawn via the plant pipeline for chunks past the LOD threshold,
    // so distant micro-model props keep their inset billboard footprint.
    pub micro_billboard_meshes: ahash::AHashMap<(i32, i32, i32), GpuPlantInstances>,
    // Phase D — micro-model LOD: chunks within this view-space forward distance
    // (world units) draw the 3D shell; beyond it, the cheap billboard. Set each
    // frame from the live render distance (`set_micro_lod_dist`); `f32::MAX` =
    // LOD off (all shells), the safe default before it's set.
    micro_lod_dist: f32,
    // Crosshair (kept as direct GPU overlay — too simple for egui)
    crosshair_pipeline: wgpu::RenderPipeline,
    crosshair_buffer: wgpu::Buffer,
    crosshair_vertex_count: u32,
    /// Cinematic Director (Phase 1) — when true, suppress the slot-0 crosshair
    /// overlay pass for a clean cinematic frame (F8 hide-HUD). Set each frame by
    /// the game loop from `hud_suppressed()`. Slot-keyed at the draw site so a
    /// split-screen player 2 keeps their crosshair.
    pub hide_crosshair: bool,
    // Block highlight wireframe
    wire_pipeline: wgpu::RenderPipeline,
    // Block-break crack overlay (Spec 05 §2.2) — textured, alpha-blended cube.
    crack_pipeline: wgpu::RenderPipeline,
    // Wallpaper face-overlay decal pipeline (owner-inbox #1/2/3) — same overlay
    // config as crack but a lit + alpha-cutout fragment (fs_decal).
    decal_pipeline: wgpu::RenderPipeline,
    // Entity rendering (textured, no back-face culling)
    pub entity_pipeline: wgpu::RenderPipeline,
    // Exhibits — hi-res painting quads (authored wall art). Each
    // carries its OWN native-resolution texture + a group-2 bind group, drawn
    // through `painting_pipeline` (vs_main + fs_painting). Filled when a world with
    // exhibits loads via `add_painting`; empty (zero cost) in every other world.
    painting_pipeline: wgpu::RenderPipeline,
    /// Exhibits — alpha-cutout variant of the painting pipeline for standing
    /// billboards (discards transparent texels). Same layout/sampler as
    /// `painting_pipeline`; the `billboards` list draws through it.
    painting_alpha_pipeline: wgpu::RenderPipeline,
    painting_bind_group_layout: wgpu::BindGroupLayout,
    painting_sampler: wgpu::Sampler,
    /// Unique artwork/plaque images keyed by id — uploaded once, shared by every
    /// quad that hangs them.
    painting_textures: ahash::AHashMap<u64, PaintingTex>,
    paintings: Vec<PaintingQuad>,
    /// Exhibits — standing billboards, re-pointed at the viewer each
    /// frame (Y-axis only) by `reorient_billboards`. Empty (zero cost) when no
    /// standing exhibit is present.
    billboards: Vec<BillboardQuad>,
    // Avatar rendering — samples a dedicated 64×64 skin texture (group 2)
    // instead of the shared block array. Not wired to any draw call yet (the
    // per-player draw lands in the next task); the pipeline/bind group are built
    // unconditionally in both constructors so a broken skin shader or a
    // bind-group-layout/`@group(2)` mismatch fails when the device builds the
    // pipeline — at real game startup (or the browser smoke test), since the
    // naga source test can't catch layout mismatches and no headless build test
    // covers this (see `wgsl_shader_validates`).
    pub avatar_pipeline: wgpu::RenderPipeline,
    pub skin_bind_group: wgpu::BindGroup,
    // Retained so Phase 3 can re-upload per-player skins into the same texture /
    // rebuild the bind group without rebuilding the pipeline.
    skin_texture: wgpu::Texture,
    #[allow(dead_code)]
    skin_bind_group_layout: wgpu::BindGroupLayout,
    /// Per-entry 3D avatar thumbnails for the "Your look" wardrobe grid, keyed
    /// by SkinId. Rendered lazily + cached; invalidated by `skin_key` change.
    skin_thumbs: std::collections::HashMap<crate::skin_wardrobe::SkinId, SkinThumb>,
    // Per-player GPU state (camera, entity mesh, wireframe)
    pub player_gpu: Vec<PlayerGpuResources>,
    // egui integration (replaces hotbar, hearts, menu, and textured UI buffers)
    pub egui: EguiIntegration,
    // Headless rendering target
    color_target: Option<wgpu::Texture>,
    /// Sky clear color [r, g, b], updated each frame for day/night cycle.
    pub sky_color: [f32; 3],
}

/// Guard the block-texture array-layer budget (Goal 3 / Task 3). The block atlas
/// allocates one array layer per registered texture (`texture_gen::texture_count()`),
/// so the device must have been granted at least that many `max_texture_array_layers`
/// or `create_texture("block_textures")` panics. Both constructors lift the requested
/// limit to the adapter's max; this catches a future block-registry wave that outgrows
/// even that — a hard panic in debug, a logged breadcrumb (before the inevitable
/// create_texture panic) in release, where `debug_assert!` is compiled out.
///
/// This should be UNREACHABLE in the browser client: the JS pre-flight gate
/// (`window.AxeWebGpu.probe()` in tools/sites/game/static/webgpu-check.js)
/// refuses to boot the WASM engine at all on an adapter whose
/// `maxTextureArrayLayers` is below its `REQUIRED_TEXTURE_ARRAY_LAYERS` floor
/// (512, see that file's comment + `texture_gen::texture_count_stays_under_webgpu_check_floor`),
/// showing the friendly "can't run yet" panel instead. If this branch fires in
/// the wild, either that gate was bypassed (native build, headless caller, a
/// future page that forgot to wire the probe) or the floor has drifted below
/// `texture_count()` — check the drift-guard test first.
fn assert_block_texture_layers_fit(num_layers: u32, device: &wgpu::Device) {
    let granted = device.limits().max_texture_array_layers;
    debug_assert!(
        num_layers <= granted,
        "block_textures needs {num_layers} array layers but the device granted only \
         {granted} (max_texture_array_layers) — lift required_limits.max_texture_array_layers \
         in request_device"
    );
    if num_layers > granted {
        log::error!(
            "block-texture array ({num_layers} layers) exceeds device \
             max_texture_array_layers ({granted}); rendering will fail — this device should have \
             been refused by the JS pre-flight gate (webgpu-check.js REQUIRED_TEXTURE_ARRAY_LAYERS) \
             before the WASM engine ever loaded"
        );
    }
}

/// Why the surface couldn't hand us a frame to draw into.
///
/// wgpu 29 folded the old `wgpu::SurfaceError` into the `CurrentSurfaceTexture`
/// enum returned by `Surface::get_current_texture`. This thin local enum keeps
/// the render loop's "reconfigure on Lost, else skip + log" contract unchanged
/// across the upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceError {
    /// Swap chain lost — reconfigure the surface before the next frame.
    Lost,
    /// Surface outdated (size/format changed under us) — reconfigure.
    Outdated,
    /// Transient (timeout / occluded / validation) — skip this frame, retry next.
    Transient,
    /// Backend reported out of memory. FLAGGED GAP: `acquire_surface_frame`
    /// below never constructs this — wgpu 29's `CurrentSurfaceTexture` has no
    /// distinct OOM case (a real backend OOM now arrives as `Validation`,
    /// which maps to `Transient` above), so `game_loop.rs`'s dedicated
    /// `Err(SurfaceError::OutOfMemory) => log::error!("GPU out of memory!")`
    /// handler is dead: a real OOM today falls through to the generic
    /// `Err(e) => log::warn!("Render error: {e:?}")` arm and loses its
    /// distinct log message. Kept (not deleted) so a human decides whether to
    /// restore OOM detection or retire this variant + its handler together.
    #[allow(dead_code)]
    OutOfMemory,
}

/// Acquire the next swap-chain frame, mapping wgpu 29's `CurrentSurfaceTexture`
/// back onto the pre-upgrade `Result<SurfaceTexture, SurfaceError>` shape so the
/// callers' frame-acquire handling reads identically.
fn acquire_surface_frame(surface: &wgpu::Surface<'_>) -> Result<wgpu::SurfaceTexture, SurfaceError> {
    match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Ok(t),
        wgpu::CurrentSurfaceTexture::Lost => Err(SurfaceError::Lost),
        wgpu::CurrentSurfaceTexture::Outdated => Err(SurfaceError::Outdated),
        wgpu::CurrentSurfaceTexture::Timeout
        | wgpu::CurrentSurfaceTexture::Occluded
        | wgpu::CurrentSurfaceTexture::Validation => Err(SurfaceError::Transient),
    }
}

/// Create the block texture array and upload every layer (Spec 03 §3.4/§3.5).
///
/// Shared by both constructors and the runtime rebuild path so the three can
/// never drift on mip levels, usage flags or upload layout. With `mipmaps`
/// false this is byte-for-byte what the engine did before Spec 39 A6:
/// `mip_level_count: 1` and one `write_texture` per layer. With it true the
/// texture is allocated with `floor(log2(size)) + 1` levels and each layer's
/// chain is generated on the CPU (alpha-weighted, in linear space — see
/// [`crate::mipmap`]) and written level by level.
///
/// `layer_count` is the array depth to allocate; `layers` supplies the pixels
/// for the first `layer_count` of them (extra layers stay zeroed).
fn create_block_texture_array<'a>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: impl Iterator<Item = &'a Vec<u8>>,
    tex_size: u32,
    layer_count: u32,
    mipmaps: bool,
) -> wgpu::Texture {
    let levels = crate::mipmap::atlas_mip_levels(tex_size, mipmaps);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("block_textures"),
        size: wgpu::Extent3d {
            width: tex_size,
            height: tex_size,
            depth_or_array_layers: layer_count,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        // CPU-generated mips need no RENDER_ATTACHMENT / STORAGE — the same two
        // usages work unchanged on WebGPU.
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (i, pixels) in layers.enumerate() {
        crate::mipmap::write_layer_with_mips(
            queue,
            &texture,
            i as u32,
            pixels,
            tex_size,
            levels,
        );
    }
    texture
}

/// The block-atlas sampler (Spec 39 A6).
///
/// - **Dial off** (default): Nearest/Nearest/Nearest — today's exact sampler.
/// - **Dial on**: `mag_filter` stays **Nearest** so pixel art is crisp up close,
///   with `min_filter` + `mipmap_filter` Linear so distant/minified faces take a
///   smoothed mip instead of aliasing. Anisotropy is deliberately **not**
///   requested: wgpu rejects `anisotropy_clamp > 1` unless *all three* filters
///   are Linear (`wgpu-core` `InvalidFilterModeWithAnisotropy`), and a Linear
///   `mag_filter` blurs every block face you are standing next to — the wrong
///   trade for a voxel game. See `docs/spec/03-rendering.md` §9.4b.
fn create_block_sampler(device: &wgpu::Device, mipmaps: bool) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("block_sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: if mipmaps {
            wgpu::FilterMode::Linear
        } else {
            wgpu::FilterMode::Nearest
        },
        mipmap_filter: if mipmaps {
            wgpu::MipmapFilterMode::Linear
        } else {
            wgpu::MipmapFilterMode::Nearest
        },
        ..Default::default()
    })
}

/// Total-pixel budget for one exhibit/gallery painting image, independent of
/// the device's per-axis `max_texture_dimension_2d` (P6 audit fix direction:
/// "cap the total pixels (e.g. 4096×4096)"). Applied in addition to the
/// per-axis clamp so a very wide-but-short image (the audit's "10000×10")
/// can't sneak a huge texture through under a generous per-axis limit.
const MAX_PAINTING_IMAGE_PIXELS: u64 = 4096 * 4096;

/// Side length of the solid-colour placeholder swapped in for a zero-size or
/// otherwise malformed painting/exhibit image.
const PLACEHOLDER_IMAGE_SIDE: u32 = 16;

/// A flat mid-grey `PLACEHOLDER_IMAGE_SIDE`×`PLACEHOLDER_IMAGE_SIDE` RGBA
/// buffer — the "image failed to decode" stand-in for `upload_painting_image`.
fn placeholder_image_rgba() -> &'static [u8] {
    // Opaque mid-grey (#808080), matching the atlas's own "missing texture"
    // tone rather than an alarming colour — this is a normal, expected path
    // (a corrupt/oversized shared world), not an error state to shout about.
    static PIXELS: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    PIXELS.get_or_init(|| {
        let n = (PLACEHOLDER_IMAGE_SIDE * PLACEHOLDER_IMAGE_SIDE) as usize;
        [128u8, 128, 128, 255].repeat(n)
    })
}

/// The width/height to upload one painting/exhibit image at, clamped to the
/// device's per-axis `max_dim` (`max_texture_dimension_2d`) and to
/// `max_pixels` total — aspect-preserving on both clamps. Returns the
/// original size unchanged when it's already within both limits. Pure so
/// it's unit-testable without a GPU device (P6 audit: "an exhibit/gallery
/// image larger than max_texture_dimension_2d crashes the renderer").
fn clamp_image_dimensions(width: u32, height: u32, max_dim: u32, max_pixels: u64) -> (u32, u32) {
    if width == 0 || height == 0 || max_dim == 0 {
        return (width, height);
    }
    let mut w = width as f64;
    let mut h = height as f64;

    // Per-axis device limit first.
    let longest = w.max(h);
    if longest > max_dim as f64 {
        let scale = max_dim as f64 / longest;
        w *= scale;
        h *= scale;
    }

    // Total-pixel budget next, on whatever the per-axis clamp left.
    let pixels = w * h;
    if pixels > max_pixels as f64 {
        let scale = (max_pixels as f64 / pixels).sqrt();
        w *= scale;
        h *= scale;
    }

    (w.floor().max(1.0) as u32, h.floor().max(1.0) as u32)
}

/// Resample an RGBA buffer from `width`×`height` to `target_w`×`target_h`
/// (bilinear — these are photos, not pixel art). Used only on the
/// already-validated "buffer matches width×height×4" path, so the
/// `RgbaImage::from_raw` here cannot fail.
fn downscale_rgba(rgba: &[u8], width: u32, height: u32, target_w: u32, target_h: u32) -> Vec<u8> {
    let Some(img) = image::RgbaImage::from_raw(width, height, rgba.to_vec()) else {
        // Unreachable on the validated call path, but never panic on image
        // data — fall back to the placeholder rather than crash.
        log::warn!("downscale_rgba: {width}x{height} buffer didn't match its own dimensions");
        return placeholder_image_rgba().to_vec();
    };
    image::imageops::resize(&img, target_w, target_h, image::imageops::FilterType::Triangle)
        .into_raw()
}

/// Native's GPU-capability gate (P6 audit: "Native has no friendly
/// GPU-capability gate: low-limit adapters panic at startup"). The web build
/// has a JS pre-flight probe (`webgpu-check.js`) that shows a friendly
/// "can't run yet" panel and never even boots the WASM engine below its
/// floor; native had nothing, so a weak adapter (old iGPU, Pi-class GLES)
/// hit a bare `.expect()` panic with no message a player could act on.
/// Logs, shows a blocking native dialog (best-effort — a headless/portal-less
/// box may have no dialog backend, hence the `eprintln!`/`log::error!`
/// baseline), then exits the process cleanly. Never returns.
#[cfg(not(target_arch = "wasm32"))]
fn native_gpu_unsupported_and_exit(reason: &str) -> ! {
    log::error!("GPU not supported, exiting: {reason}");
    eprintln!("AxeNStax: this device's graphics hardware isn't supported.\n{reason}");
    // BRIDGE: Android has no rfd backend (see Cargo.toml), so an APK on an
    // unsupported GPU closes with the reason in logcat only. Replace with an
    // on-screen message when the Android port gets a native dialog path.
    #[cfg(not(target_os = "android"))]
    rfd::MessageDialog::new()
        .set_title("AxeNStax — graphics not supported")
        .set_description(reason)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
    std::process::exit(1);
}

impl Renderer {
    pub async fn new(window: Arc<Window>, textures: &[Vec<u8>]) -> Self {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        log::info!("Creating wgpu instance...");
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            #[cfg(not(target_arch = "wasm32"))]
            backends: wgpu::Backends::all(),
            #[cfg(target_arch = "wasm32")]
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });

        log::info!("Creating surface...");
        let surface = instance.create_surface(window.clone()).unwrap();

        log::info!("Requesting GPU adapter...");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await;
        // Native has no equivalent of the web's JS pre-flight gate
        // (`webgpu-check.js`), which refuses to boot the WASM engine at all
        // below its `maxTextureArrayLayers` floor — a low-limit adapter (old
        // iGPU, Pi-class GLES) used to panic straight out of `.expect()`
        // with no message the player could act on (P6 audit). Fail into a
        // readable dialog + clean exit instead, on every native failure
        // mode below (no adapter, adapter too weak, device request denied).
        #[cfg(not(target_arch = "wasm32"))]
        let adapter = match adapter {
            Ok(a) => a,
            Err(e) => native_gpu_unsupported_and_exit(&format!(
                "No compatible graphics adapter was found on this device: {e}"
            )),
        };
        #[cfg(target_arch = "wasm32")]
        let adapter = adapter.expect("Failed to find a suitable GPU adapter");

        log::info!("GPU adapter: {:?}", adapter.get_info().name);

        // Native capability gate — mirrors the web pre-flight probe's floor.
        // The block-texture atlas needs one array layer per registered block
        // texture; below that, `request_device` may still succeed (it only
        // grants what we *ask* for) but the later `create_texture` for the
        // block atlas would panic on validation. Catch it here, before ever
        // touching the device, with a message a player can actually read.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let needed = texture_gen::texture_count();
            let granted = adapter.limits().max_texture_array_layers;
            if granted < needed {
                native_gpu_unsupported_and_exit(&format!(
                    "This device's graphics adapter supports only {granted} texture array \
                     layers, but AxeNStax needs at least {needed}. A newer graphics driver or \
                     GPU is required."
                ));
            }
        }

        log::info!("Requesting GPU device...");
        let device_result = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("axenstax_device"),
                required_features: wgpu::Features::empty(),
                // Downlevel limits, raised to whatever this adapter actually
                // advertises, on BOTH platforms — not `Limits::default()` on
                // native. `Limits::default()` assumes a modern desktop-class
                // adapter; requesting it against a GL/GLES adapter (old iGPU,
                // Pi-class, a software rasterizer) that falls under those
                // defaults makes `request_device` itself fail (P6 audit fix
                // direction: "request `Limits::downlevel_defaults()
                // .using_resolution(adapter.limits())`"). `max_texture_array_layers`
                // is then lifted back up separately — `downlevel_defaults()`
                // never grants more than 256, and the block atlas needs 506+.
                //
                // BUT: `using_resolution` only forwards max_texture_dimension_{1d,2d,3d}
                // from the adapter — it does NOT lift `max_texture_array_layers`. The
                // block-texture atlas grows one layer per registered block; once we
                // crossed 256 (the WebGL2-downlevel default) Chromium rejected the
                // atlas with "exceeded maximum texture size [..., depthOrArrayLayers:256]"
                // and the whole render chain went black. Pin the array-layers limit
                // to whatever the adapter advertises (modern Chromium: 2048).
                #[cfg(not(target_arch = "wasm32"))]
                required_limits: {
                    let base = wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits());
                    wgpu::Limits {
                        max_texture_array_layers: adapter.limits().max_texture_array_layers,
                        ..base
                    }
                },
                #[cfg(target_arch = "wasm32")]
                required_limits: {
                    let base = wgpu::Limits::downlevel_webgl2_defaults()
                        .using_resolution(adapter.limits());
                    wgpu::Limits {
                        max_texture_array_layers: adapter.limits().max_texture_array_layers,
                        ..base
                    }
                },
                ..Default::default()
            })
            .await;
        #[cfg(not(target_arch = "wasm32"))]
        let (device, queue) = match device_result {
            Ok(dq) => dq,
            Err(e) => native_gpu_unsupported_and_exit(&format!(
                "Failed to create a graphics device on this adapter: {e}"
            )),
        };
        #[cfg(target_arch = "wasm32")]
        let (device, queue) = device_result.expect("Failed to create GPU device");

        log::info!("GPU device created successfully");

        let surface_caps = surface.get_capabilities(&adapter);
        // `surface_format` is the (sRGB) format every pipeline + view renders
        // through; `config_format` is what the canvas is configured with (differs
        // from `surface_format` only on WebGPU). See `choose_surface_formats`.
        let (config_format, surface_format, surface_view_formats) =
            choose_surface_formats(&surface_caps.formats);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: config_format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: surface_view_formats,
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let format = surface_format;

        let depth_texture_view = create_depth_texture(&device, width, height);

        // --- Camera uniform ---
        let camera_uniform = CameraUniform {
            view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
            camera_pos: [0.0, 0.0, 0.0, 0.0],
            sun_dir: [0.3, 1.0, 0.5, 1.0],
            fog: crate::camera::default_fog(),
            params: [1.0, 0.0, 0.0, 0.0],
        cam_right: [1.0, 0.0, 0.0, 0.0],
        cam_up: [0.0, 1.0, 0.0, 0.0],
        };
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera_uniform"),
            contents: bytemuck::cast_slice(&[camera_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("camera_bind_group_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera_bind_group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        // --- Block texture array ---
        // Atlas resolution is inferred from the layer bytes (Spec 03 §11.3): a
        // resolution-agnostic pack hands us pre-scaled res×res layers.
        let tex_size = textures
            .first()
            .map_or(16, |t| crate::texture_registry::square_side(t.len()));
        let num_layers = texture_gen::texture_count();
        assert_block_texture_layers_fit(num_layers, &device);

        // Mipmaps start OFF (today's look); `set_mipmaps` rebuilds the array +
        // sampler live when the Graphics dial turns them on (Spec 39 A6).
        let block_mipmaps = false;
        let block_texture = create_block_texture_array(
            &device,
            &queue,
            textures.iter(),
            tex_size,
            num_layers,
            block_mipmaps,
        );

        let block_texture_view = block_texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let block_sampler = create_block_sampler(&device, block_mipmaps);

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("texture_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("texture_bind_group"),
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&block_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&block_sampler),
                },
            ],
        });

        // --- Main chunk pipeline (with texture bind group) ---
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("chunk_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("chunk_pipeline_layout"),
            bind_group_layouts: &[Some(&camera_bind_group_layout), Some(&texture_bind_group_layout)],
            immediate_size: 0,
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("chunk_render_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Vertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // --- Entity pipeline (same as chunk but NO back-face culling) ---
        let entity_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("entity_render_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Vertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // --- Avatar pipeline (64×64 skin texture, group 2). Unused by draw
        // calls until the next task; built here so a broken skin shader or a
        // bind-group-layout/`@group(2)` mismatch fails at device pipeline
        // creation (real startup / browser smoke test), not silently later. ---
        let AvatarResources {
            skin_texture,
            skin_bind_group_layout,
            skin_bind_group,
            avatar_pipeline,
        } = create_avatar_resources(
            &device,
            &queue,
            surface_format,
            &block_sampler,
            &camera_bind_group_layout,
            &texture_bind_group_layout,
        );

        let (
            painting_pipeline,
            painting_alpha_pipeline,
            painting_bind_group_layout,
            painting_sampler,
        ) = create_painting_resources(
                &device,
                &shader,
                surface_format,
                &camera_bind_group_layout,
                &texture_bind_group_layout,
            );

        // --- Water pipeline (alpha blended, depth read-only) ---
        let water_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water_render_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Vertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_water"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // Water visible from both sides (swimming underwater)
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // #131 — transparent-solid pipeline (glass, …); mirrors water + fs_transparent.
        let transparent_pipeline =
            build_transparent_pipeline(&device, &shader, &pipeline_layout, surface_format);

        // --- egui integration (replaces old UI pipeline, hotbar, hearts, menu rendering) ---
        let mut egui = EguiIntegration::new(&device, surface_format, &window);

        // Upload block textures as egui managed textures for inventory display
        egui.upload_block_textures(&device, &queue, textures, 16);

        // --- Overlay shader (crosshair, wireframe) ---
        let overlay_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("overlay_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("overlay.wgsl").into()),
        });

        // --- Crosshair pipeline ---
        let crosshair_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("crosshair_pipeline_layout"),
                bind_group_layouts: &[],
                immediate_size: 0,
            });

        let crosshair_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("crosshair_pipeline"),
                layout: Some(&crosshair_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &overlay_shader,
                    entry_point: Some("vs_crosshair"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<CrosshairVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &overlay_shader,
                    entry_point: Some("fs_crosshair"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        let crosshair_buffer = create_crosshair_buffer(&device, config.width, config.height);

        // --- Wireframe pipeline ---
        let wire_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("wire_pipeline_layout"),
                bind_group_layouts: &[Some(&camera_bind_group_layout)],
                immediate_size: 0,
            });

        let wire_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("wire_pipeline"),
                layout: Some(&wire_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &overlay_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<WireVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &overlay_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState {
                        constant: -2,
                        slope_scale: -1.0,
                        clamp: 0.0,
                    },
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        // Active pack's animated textures (Spec 03 §3.4); empty for the default
        // pack. block_texture is retained on Self so these can update per frame.
        let animated_textures = Self::load_active_animations(tex_size);

        let crack_pipeline = make_overlay_pipeline(
            &device, &shader, &pipeline_layout, surface_format, "fs_crack", "crack_render_pipeline",
        );
        let decal_pipeline = make_overlay_pipeline(
            &device, &shader, &pipeline_layout, surface_format, "fs_decal", "decal_render_pipeline",
        );

        let plant_pipeline = create_plant_pipeline(&device, &pipeline_layout, &shader, surface_format);
        let (plant_geo_vbuf, plant_geo_ibuf, plant_geo_index_count) = create_plant_geo(&device);
        let (particle_pipeline, particle_geo_vbuf, particle_geo_ibuf, particle_geo_index_count, particle_instance_buffer) =
            create_particle_gpu(&device, &pipeline_layout, &shader, surface_format);
        let micro_pipeline = create_micro_pipeline(&device, &pipeline_layout, &shader, surface_format);
        // Render-scale blit (Spec 39 Phase 5). Offscreen target is created lazily
        // by set_render_scale when scale < 1.0.
        let (blit_pipeline, blit_sampler, blit_bind_group_layout) =
            build_blit_resources(&device, &shader, surface_format);

        let (vm_cam_buf, vm_cam_bg) =
            make_viewmodel_camera(&device, &camera_bind_group_layout);

        Self {
            surface: Some(surface),
            instance,
            device,
            queue,
            format,
            config_format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            render_pipeline,
            depth_texture_view,
            camera_bind_group_layout,
            texture_bind_group,
            block_sampler,
            texture_bind_group_layout,
            animated_last_frame: vec![u32::MAX; animated_textures.len()],
            animated_textures,
            block_texture,
            block_tex_size: tex_size,
            block_mipmaps,
            anim_clock: 0,
            chunk_meshes: ahash::AHashMap::new(),
            water_pipeline,
            water_meshes: ahash::AHashMap::new(),
            transparent_pipeline,
            transparent_meshes: ahash::AHashMap::new(),
            plant_pipeline,
            plant_geo_vbuf,
            plant_geo_ibuf,
            plant_geo_index_count,
            plant_meshes: ahash::AHashMap::new(),
            particle_pipeline,
            particle_geo_vbuf,
            particle_geo_ibuf,
            particle_geo_index_count,
            particle_instance_buffer,
            particle_count: 0,
            decal_meshes: ahash::AHashMap::new(),
            micro_pipeline,
            micro_geo: ahash::AHashMap::new(),
            micro_meshes: ahash::AHashMap::new(),
            micro_billboard_meshes: ahash::AHashMap::new(),
            micro_lod_dist: f32::MAX,
            crosshair_pipeline,
            crosshair_buffer,
            crosshair_vertex_count: 12,
            hide_crosshair: false,
            wire_pipeline,
            crack_pipeline,
            decal_pipeline,
            entity_pipeline,
            painting_pipeline,
            painting_alpha_pipeline,
            painting_bind_group_layout,
            painting_sampler,
            painting_textures: ahash::AHashMap::new(),
            paintings: Vec::new(),
            billboards: Vec::new(),
            avatar_pipeline,
            skin_bind_group,
            skin_texture,
            skin_bind_group_layout,
            skin_thumbs: std::collections::HashMap::new(),
            player_gpu: vec![PlayerGpuResources {
                camera_buffer,
                camera_bind_group,
                entity_buffer: None,
                entity_vertex_count: 0,
                avatar_buffer: None,
                avatar_vertex_count: 0,
                wire_buffer: None,
                wire_vertex_count: 0,
                crack_buffer: None,
                crack_vertex_count: 0,
                ghost_buffer: None,
                ghost_vertex_count: 0,
                spawn_marker_buffer: None,
                spawn_marker_vertex_count: 0,
                build_guide_buffer: None,
                build_guide_vertex_count: 0,
                trial_ghost_buffer: None,
                trial_ghost_vertex_count: 0,
                beam_buffer: None,
                beam_vertex_count: 0,
                paint_aid_buffer: None,
                paint_aid_vertex_count: 0,
                viewmodel_buffer: None,
                viewmodel_vertex_count: 0,
                viewmodel_skin_buffer: None,
                viewmodel_skin_vertex_count: 0,
                viewmodel_camera_buffer: vm_cam_buf,
                viewmodel_camera_bind_group: vm_cam_bg,
                last_view_proj: glam::Mat4::IDENTITY,
            }],
            last_draw_stats: DrawStats::default(),
            crosshair_cache: ahash::AHashMap::new(),
            render_scale: 1.0,
            offscreen_color_view: None,
            offscreen_depth_view: None,
            offscreen_bind_group: None,
            blit_pipeline,
            blit_sampler,
            blit_bind_group_layout,
            egui,
            color_target: None,
            sky_color: [0.53, 0.72, 0.90], // Default: noon sky
        }
    }

    /// The surface's `view_formats` for (re)configuration: the sRGB render-format
    /// alias when it differs from the canvas config format (WebGPU), else empty
    /// (native). Must match the alias used to create the per-frame surface view.
    fn surface_view_formats(&self) -> Vec<wgpu::TextureFormat> {
        if self.format != self.config_format {
            vec![self.format]
        } else {
            vec![]
        }
    }

    /// Rebuild the surface if Android threw it away, immediately before it is
    /// needed.
    ///
    /// Doing this at RENDER time rather than on `resumed` is deliberate: on a
    /// Pixel 8 (the 2026-07 spike), winit's Android backend fired `resumed()`
    /// exactly once at startup and never again across background/foreground
    /// cycles, so a rebuild-on-resume never ran and the app froze on its last
    /// frame. Rebuilding here is self-healing and independent of lifecycle
    /// event ordering — whoever needs a surface gets one.
    ///
    /// No-op on every other platform: only Android revokes a live window.
    #[allow(unused_variables)]
    fn ensure_surface(&mut self, window: &Arc<Window>) {
        #[cfg(target_os = "android")]
        if self.surface.is_none() {
            log::info!("surface missing at render time — rebuilding");
            self.recreate_surface(window.clone());
        }
    }

    /// Rebuild the swapchain surface for a freshly-created native window.
    ///
    /// Reuses the cached `config_format` / `present_mode` / view formats rather
    /// than re-querying capabilities: the adapter has not changed, only the
    /// window, and re-deriving them risks picking a different format from the
    /// one every pipeline was already built against. If the window is still
    /// gone (winit gives a null handle between Suspended and Resumed) creation
    /// fails, is logged, and the caller skips the frame and retries next time.
    ///
    /// Only reached from `ensure_surface`, whose body is Android-only; left
    /// compiled on all platforms so a desktop build still type-checks it.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    fn recreate_surface(&mut self, window: Arc<Window>) {
        let surface = match self.instance.create_surface(window) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("could not recreate surface (window not back yet?): {e}");
                return;
            }
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: self.config_format,
            width: self.width,
            height: self.height,
            present_mode: self.present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: self.surface_view_formats(),
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&self.device, &config);
        self.surface = Some(surface);
        log::info!("surface recreated ({}x{})", self.width, self.height);
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.width = width;
            self.height = height;
            // Viewport sizes change on resize — drop cached crosshair buffers
            // so they're rebuilt at the new size (and stale sizes don't pile up).
            self.crosshair_cache.clear();
            if let Some(surface) = &self.surface {
                let config = wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format: self.config_format,
                    width,
                    height,
                    present_mode: self.present_mode,
                    alpha_mode: wgpu::CompositeAlphaMode::Auto,
                    view_formats: self.surface_view_formats(),
                    desired_maximum_frame_latency: 2,
                };
                surface.configure(&self.device, &config);
            }
            self.depth_texture_view = create_depth_texture(&self.device, width, height);
            // Render-scale offscreen target follows the surface size (Spec 39 P5).
            self.recreate_offscreen();
        }
    }

    /// Set the surface present mode (Spec 39 — vsync vs uncapped) and reconfigure
    /// the surface in place. No-op when the mode is unchanged or there's no
    /// surface (headless). The software FPS cap (`FrameLimit::Cap`) is applied by
    /// the game-loop frame pacer, not here — those map to `AutoNoVsync`.
    pub fn set_present_mode(&mut self, mode: wgpu::PresentMode) {
        if mode == self.present_mode {
            return;
        }
        self.present_mode = mode;
        if let Some(surface) = &self.surface {
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: self.config_format,
                width: self.width,
                height: self.height,
                present_mode: self.present_mode,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: self.surface_view_formats(),
                desired_maximum_frame_latency: 2,
            };
            surface.configure(&self.device, &config);
        }
    }

    /// Owner-inbox #18 Phase D — set the micro-model LOD distance (view-space
    /// forward distance in world units). Beyond it, micro-models draw their cheap
    /// billboard instead of the 3D shell. Call each frame from the live render
    /// distance, e.g. `render_distance_chunks * CHUNK_SIZE * 0.55`.
    pub fn set_micro_lod_dist(&mut self, dist: f32) {
        self.micro_lod_dist = dist.max(0.0);
    }

    /// Set the render scale (Spec 39 Phase 5). 1.0 renders the world straight to
    /// the surface (no offscreen, no blit cost); < 1.0 renders the world to a
    /// smaller offscreen target then upscales it — the biggest weak-GPU lever.
    /// Recreates the offscreen target at the new size; UI still draws at native.
    pub fn set_render_scale(&mut self, scale: f32) {
        let scale = scale.clamp(crate::graphics_settings::RENDER_SCALE_MIN, 1.0);
        if (scale - self.render_scale).abs() < 1e-4 {
            return;
        }
        self.render_scale = scale;
        self.recreate_offscreen();
    }

    /// (Re)build the offscreen world target to match the current size + scale.
    /// At scale 1.0 the offscreen is dropped (direct-to-surface path). Called on
    /// scale change and on resize.
    fn recreate_offscreen(&mut self) {
        if self.render_scale >= 0.999 {
            self.offscreen_color_view = None;
            self.offscreen_depth_view = None;
            self.offscreen_bind_group = None;
            return;
        }
        let w = ((self.width as f32) * self.render_scale).round().max(1.0) as u32;
        let h = ((self.height as f32) * self.render_scale).round().max(1.0) as u32;
        let (cv, dv, bg) = create_offscreen_target(
            &self.device,
            self.format,
            &self.blit_bind_group_layout,
            &self.blit_sampler,
            w,
            h,
        );
        self.offscreen_color_view = Some(cv);
        self.offscreen_depth_view = Some(dv);
        self.offscreen_bind_group = Some(bg);
    }

    pub fn update_camera(&mut self, player_index: usize, uniform: &CameraUniform) {
        // Distinct-field split borrow: &self.queue + &mut self.player_gpu.
        let queue = &self.queue;
        if let Some(gpu) = self.player_gpu.get_mut(player_index) {
            queue.write_buffer(&gpu.camera_buffer, 0, bytemuck::cast_slice(&[*uniform]));
            // Cache the CPU-side view-proj for this frame's frustum culling (A1).
            gpu.last_view_proj = glam::Mat4::from_cols_array_2d(&uniform.view_proj);
        }
    }

    pub fn create_player_resources(&self) -> PlayerGpuResources {
        let camera_uniform = crate::camera::CameraUniform {
            view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
            camera_pos: [0.0, 0.0, 0.0, 0.0],
            sun_dir: [0.3, 1.0, 0.5, 1.0],
            fog: crate::camera::default_fog(),
            params: [1.0, 0.0, 0.0, 0.0],
        cam_right: [1.0, 0.0, 0.0, 0.0],
        cam_up: [0.0, 1.0, 0.0, 0.0],
        };
        let camera_buffer = self.device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("player_camera_uniform"),
                contents: bytemuck::cast_slice(&[camera_uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            },
        );
        let camera_bind_group = self.device.create_bind_group(
            &wgpu::BindGroupDescriptor {
                label: Some("player_camera_bind_group"),
                layout: &self.camera_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                }],
            },
        );
        let (viewmodel_camera_buffer, viewmodel_camera_bind_group) =
            make_viewmodel_camera(&self.device, &self.camera_bind_group_layout);
        PlayerGpuResources {
            camera_buffer,
            camera_bind_group,
            entity_buffer: None,
            entity_vertex_count: 0,
            avatar_buffer: None,
            avatar_vertex_count: 0,
            wire_buffer: None,
            wire_vertex_count: 0,
            crack_buffer: None,
            crack_vertex_count: 0,
            ghost_buffer: None,
            ghost_vertex_count: 0,
            spawn_marker_buffer: None,
            spawn_marker_vertex_count: 0,
            build_guide_buffer: None,
            build_guide_vertex_count: 0,
            trial_ghost_buffer: None,
            trial_ghost_vertex_count: 0,
            beam_buffer: None,
            beam_vertex_count: 0,
            paint_aid_buffer: None,
            paint_aid_vertex_count: 0,
            viewmodel_buffer: None,
            viewmodel_vertex_count: 0,
            viewmodel_skin_buffer: None,
            viewmodel_skin_vertex_count: 0,
            viewmodel_camera_buffer,
            viewmodel_camera_bind_group,
            last_view_proj: glam::Mat4::IDENTITY,
        }
    }

    pub fn upload_chunk_mesh(&mut self, pos: (i32, i32, i32), mesh: &ChunkMesh) {
        if mesh.vertices.is_empty() {
            self.chunk_meshes.remove(&pos);
            return;
        }

        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("chunk_vertex_buffer"),
                contents: bytemuck::cast_slice(&mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });

        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("chunk_index_buffer"),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });

        self.chunk_meshes.insert(
            pos,
            GpuChunkMesh {
                vertex_buffer,
                index_buffer,
                index_count: mesh.indices.len() as u32,
            },
        );
    }

    pub fn upload_water_mesh(&mut self, pos: (i32, i32, i32), mesh: &ChunkMesh) {
        if mesh.vertices.is_empty() {
            self.water_meshes.remove(&pos);
            return;
        }

        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("water_vertex_buffer"),
                contents: bytemuck::cast_slice(&mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });

        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("water_index_buffer"),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });

        self.water_meshes.insert(
            pos,
            GpuChunkMesh {
                vertex_buffer,
                index_buffer,
                index_count: mesh.indices.len() as u32,
            },
        );
    }

    /// #131 — upload a chunk's solid+transparent (glass, …) mesh. An empty mesh
    /// clears the chunk's entry. Mirrors `upload_water_mesh`.
    pub fn upload_transparent_mesh(&mut self, pos: (i32, i32, i32), mesh: &ChunkMesh) {
        if mesh.vertices.is_empty() {
            self.transparent_meshes.remove(&pos);
            return;
        }
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("transparent_vertex_buffer"),
                contents: bytemuck::cast_slice(&mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("transparent_index_buffer"),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        self.transparent_meshes.insert(
            pos,
            GpuChunkMesh {
                vertex_buffer,
                index_buffer,
                index_count: mesh.indices.len() as u32,
            },
        );
    }

    /// Upload a chunk's instanced plants (flowers/grass/crops). An empty
    /// slice clears the chunk's plant buffer.
    /// Particle framework (2026-07-05): rewrite the persistent instance
    /// buffer's prefix with this frame's live particles.
    pub fn upload_particles(&mut self, instances: &[crate::mesh::ParticleInstance]) {
        let n = instances.len().min(crate::particles::CAP_FULL);
        if n > 0 {
            self.queue.write_buffer(
                &self.particle_instance_buffer,
                0,
                bytemuck::cast_slice(&instances[..n]),
            );
        }
        self.particle_count = n as u32;
    }

    pub fn upload_plant_instances(
        &mut self,
        pos: (i32, i32, i32),
        instances: &[crate::mesh::PlantInstance],
    ) {
        if instances.is_empty() {
            self.plant_meshes.remove(&pos);
            return;
        }
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("plant_instance_buffer"),
                contents: bytemuck::cast_slice(instances),
                usage: wgpu::BufferUsages::VERTEX,
            });
        self.plant_meshes.insert(
            pos,
            GpuPlantInstances {
                buffer,
                count: instances.len() as u32,
            },
        );
    }

    /// Upload all three GPU representations for a chunk at once — opaque
    /// mesh, water mesh, and instanced plants. Single entry point so no
    /// call site can forget the plant instances (added 2026-05-30).
    pub fn upload_chunk(&mut self, pos: (i32, i32, i32), meshes: &crate::mesh::ChunkMeshes) {
        self.upload_chunk_mesh(pos, &meshes.opaque);
        self.upload_water_mesh(pos, &meshes.water);
        self.upload_transparent_mesh(pos, &meshes.transparent);
        self.upload_plant_instances(pos, &meshes.plants);
        self.upload_chunk_decals(pos, &meshes.decals);
        self.upload_micro_instances(pos, &meshes.micro_instances);
        self.upload_micro_billboards(pos, &meshes.micro_billboards);
    }

    /// Owner-inbox #1/2/3 — upload a chunk's wallpaper decal quads (a non-indexed
    /// triangle list of `mesh::Vertex`). Empty => drop the entry, mirroring
    /// `upload_plant_instances`. Called from `upload_chunk` so no remesh site
    /// can forget it.
    pub fn upload_chunk_decals(&mut self, pos: (i32, i32, i32), decals: &[crate::mesh::Vertex]) {
        if decals.is_empty() {
            self.decal_meshes.remove(&pos);
            return;
        }
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("decal_vertex_buffer"),
                contents: bytemuck::cast_slice(decals),
                usage: wgpu::BufferUsages::VERTEX,
            });
        self.decal_meshes.insert(
            pos,
            GpuDecalMesh {
                vertex_buffer,
                vertex_count: decals.len() as u32,
            },
        );
    }

    /// Exhibits / paintings — upload one unique artwork/plaque image under `tex_id` (its
    /// own native-resolution texture + group-2 bind group). Idempotent per id, so
    /// an image shared by many walls is uploaded once. Ignores malformed buffers.
    pub fn upload_painting_image(&mut self, tex_id: u64, width: u32, height: u32, rgba: &[u8]) {
        // Zero-size or a buffer that's too short for its claimed dimensions
        // (P6 audit: "reject zero-size or undecodable images with a
        // placeholder") — never call `create_texture` on it, and never leave
        // the exhibit slot silently empty either. A small solid-colour tile
        // stands in so the quad still draws something recognisable instead
        // of nothing.
        let (width, height, rgba) = if width == 0
            || height == 0
            || rgba.len() < (width as usize * height as usize * 4)
        {
            log::warn!(
                "upload_painting_image: rejecting malformed image for tex {tex_id} \
                 ({width}x{height}, {} bytes) — using a placeholder",
                rgba.len()
            );
            (
                PLACEHOLDER_IMAGE_SIDE,
                PLACEHOLDER_IMAGE_SIDE,
                std::borrow::Cow::Borrowed(placeholder_image_rgba()),
            )
        } else {
            // Clamp to the device's per-axis limit AND a total-pixel budget
            // (P6 audit: "an exhibit/gallery image larger than
            // max_texture_dimension_2d crashes the renderer" — a shared
            // `.axeworld` is enough to trigger it on every entry). Downscale
            // rather than reject so a legitimately large photo still hangs,
            // just capped.
            let max_dim = self.device.limits().max_texture_dimension_2d;
            let (target_w, target_h) =
                clamp_image_dimensions(width, height, max_dim, MAX_PAINTING_IMAGE_PIXELS);
            if target_w == width && target_h == height {
                (width, height, std::borrow::Cow::Borrowed(rgba))
            } else {
                log::warn!(
                    "upload_painting_image: downscaling tex {tex_id} from {width}x{height} to \
                     {target_w}x{target_h} (device max_texture_dimension_2d={max_dim}, pixel \
                     budget={MAX_PAINTING_IMAGE_PIXELS})"
                );
                (
                    target_w,
                    target_h,
                    std::borrow::Cow::Owned(downscale_rgba(rgba, width, height, target_w, target_h)),
                )
            }
        };
        let rgba: &[u8] = rgba.as_ref();
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("painting_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("painting_bind_group"),
            layout: &self.painting_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.painting_sampler),
                },
            ],
        });
        self.painting_textures.insert(
            tex_id,
            PaintingTex {
                bind_group,
                _texture: texture,
            },
        );
    }

    /// Exhibits / paintings — hang one quad showing image `tex_id` (uploaded separately).
    /// `center`/`normal` are world-space; `height` is in blocks and `aspect`
    /// (= image width / height) sets the width so art is never stretched. A
    /// consistent `right = up × normal` basis keeps the image unmirrored. The
    /// quad draws once its texture has been uploaded; order doesn't matter.
    pub fn add_painting_quad(
        &mut self,
        tex_id: u64,
        center: [f32; 3],
        normal: [f32; 3],
        height: f32,
        aspect: f32,
    ) {
        let hh = height * 0.5;
        let hw = height * aspect * 0.5;
        let center = glam::Vec3::from(center);
        let normal = glam::Vec3::from(normal).normalize_or_zero();
        let up = glam::Vec3::Y;
        let right = up.cross(normal).normalize_or_zero();
        let bl = center - right * hw - up * hh;
        let br = center + right * hw - up * hh;
        let tr = center + right * hw + up * hh;
        let tl = center - right * hw + up * hh;
        let n = [normal.x, normal.y, normal.z];
        let mk = |p: glam::Vec3, uv: [f32; 2]| crate::mesh::Vertex {
            position: [p.x, p.y, p.z],
            normal: n,
            tex_layer: 0,
            uv,
            light: 1.0,
            sky_light: 0.0,
        };
        // UV origin top-left; quad corners bl, br, tr, tl.
        let verts = [
            mk(bl, [0.0, 1.0]),
            mk(br, [1.0, 1.0]),
            mk(tr, [1.0, 0.0]),
            mk(tl, [0.0, 0.0]),
        ];
        let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("painting_vertex_buffer"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("painting_index_buffer"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        let aabb_min = bl.min(br).min(tr).min(tl);
        let aabb_max = bl.max(br).max(tr).max(tl);
        self.paintings.push(PaintingQuad {
            tex_id,
            vertex_buffer,
            index_buffer,
            index_count: 6,
            aabb_min,
            aabb_max,
        });
    }

    /// Exhibits — register a standing **billboard** showing image `tex_id`
    /// (uploaded via `upload_painting_image`). `anchor` is the world-space quad
    /// centre; `height` (blocks) + `aspect` (image w/h) size it. The quad is
    /// initialised facing +Z and re-pointed at the viewer every frame by
    /// `reorient_billboards`. Drawn through the alpha painting pass (cut-outs).
    pub fn add_billboard_quad(
        &mut self,
        tex_id: u64,
        anchor: [f32; 3],
        height: f32,
        aspect: f32,
    ) {
        let anchor = glam::Vec3::from(anchor);
        let (verts, aabb_min, aabb_max) =
            painting_quad_verts(anchor, glam::Vec3::Z, height, aspect);
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("billboard_vertex_buffer"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
        let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("billboard_index_buffer"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        self.billboards.push(BillboardQuad {
            tex_id,
            anchor,
            height,
            aspect,
            vertex_buffer,
            index_buffer,
            index_count: 6,
            aabb_min,
            aabb_max,
        });
    }

    /// Exhibits — re-point every standing billboard toward `eye` (Y-axis only) and
    /// rewrite its vertex buffer in place. Cheap (one `write_buffer` per billboard,
    /// no reallocation); call once per frame BEFORE the draw. No-op when there are
    /// no billboards.
    pub fn reorient_billboards(&mut self, eye: [f32; 3]) {
        for b in &mut self.billboards {
            let yaw = crate::exhibit::billboard_yaw(b.anchor.into(), eye);
            let normal = glam::Vec3::from(crate::exhibit::yaw_to_normal(yaw));
            let (verts, aabb_min, aabb_max) =
                painting_quad_verts(b.anchor, normal, b.height, b.aspect);
            b.aabb_min = aabb_min;
            b.aabb_max = aabb_max;
            self.queue
                .write_buffer(&b.vertex_buffer, 0, bytemuck::cast_slice(&verts));
        }
    }

    /// Exhibits / paintings — drop every hung quad AND its textures, so art never bleeds
    /// between worlds.
    pub fn clear_paintings(&mut self) {
        self.paintings.clear();
        self.billboards.clear();
        self.painting_textures.clear();
    }

    /// Exhibits — drop the hung quads but KEEP the uploaded textures, so the
    /// exhibit lifecycle can rebuild quads at the real image aspect once decode
    /// reveals it (no re-fetch / re-upload). Used by `tick_exhibit_art`.
    pub fn clear_painting_quads(&mut self) {
        self.paintings.clear();
        self.billboards.clear();
    }


    /// Owner-inbox #18 — upload a chunk's micro-model instances, grouped by block
    /// type so each type draws against its own shared baked shell. Empty => drop
    /// the entry, mirroring `upload_plant_instances`. Per-type shell geometry is
    /// uploaded separately + once by `sync_micro_models`.
    pub fn upload_micro_instances(
        &mut self,
        pos: (i32, i32, i32),
        instances: &[(crate::block::BlockId, crate::mesh::MicroInstance)],
    ) {
        if instances.is_empty() {
            self.micro_meshes.remove(&pos);
            return;
        }
        let groups = crate::mesh::group_micro_instances(instances);
        let mut batches = Vec::with_capacity(groups.len());
        for (block_id, insts) in groups {
            let buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("micro_instance_buffer"),
                    contents: bytemuck::cast_slice(&insts),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            batches.push((block_id, GpuPlantInstances { buffer, count: insts.len() as u32 }));
        }
        self.micro_meshes.insert(pos, batches);
    }

    /// Owner-inbox #18 Phase D — upload a chunk's far-LOD fallback billboards (a
    /// plain `PlantInstance` buffer, drawn via the plant pipeline + shared cross).
    /// Empty => drop the entry, mirroring `upload_plant_instances`.
    pub fn upload_micro_billboards(
        &mut self,
        pos: (i32, i32, i32),
        instances: &[crate::mesh::PlantInstance],
    ) {
        if instances.is_empty() {
            self.micro_billboard_meshes.remove(&pos);
            return;
        }
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("micro_billboard_buffer"),
                contents: bytemuck::cast_slice(instances),
                usage: wgpu::BufferUsages::VERTEX,
            });
        self.micro_billboard_meshes
            .insert(pos, GpuPlantInstances { buffer, count: instances.len() as u32 });
    }

    /// Owner-inbox #18 — upload one shared baked shell geometry per registered
    /// micro-model type, from `World::micro_registry`. Idempotent: only uploads
    /// types NOT already present, so it's cheap to call on each world load. Note:
    /// RE-baking an already-uploaded type (same block id, new geometry) is NOT
    /// picked up here — call `clear_micro_geo` first. Registration is startup-only
    /// today, and `reset_for_world_change` clears the geo on world switch, so this
    /// only matters for a future live re-bake / world-specific (Stash) registry.
    /// The per-chunk instance buffers from `upload_micro_instances` draw against these.
    pub fn sync_micro_models(
        &mut self,
        registry: &crate::micro_model_registry::MicroModelRegistry,
    ) {
        use wgpu::util::DeviceExt;
        for (block_id, baked) in registry.iter() {
            if self.micro_geo.contains_key(block_id) || baked.mesh.indices.is_empty() {
                continue;
            }
            let vertex_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("micro_geo_vbuf"),
                    contents: bytemuck::cast_slice(&baked.mesh.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            let index_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("micro_geo_ibuf"),
                    contents: bytemuck::cast_slice(&baked.mesh.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
            self.micro_geo.insert(
                *block_id,
                GpuChunkMesh {
                    vertex_buffer,
                    index_buffer,
                    index_count: baked.mesh.indices.len() as u32,
                },
            );
        }
    }

    /// Owner-inbox #18 — drop all uploaded per-type micro-model shell geometry.
    /// Called by `reset_for_world_change` so a new world re-syncs fresh geometry
    /// (and so a future re-bake of a registered type is picked up after a
    /// `sync_micro_models`).
    pub fn clear_micro_geo(&mut self) {
        self.micro_geo.clear();
    }

    /// Spec 24 Phase 8 — upload the ghost-placement wireframe for a
    /// player. `positions` are world-space block coordinates; `color`
    /// is the RGB tint (alpha is fixed at 0.75 so the lines read
    /// strongly against most terrain). An empty slice clears the
    /// ghost buffer.
    pub fn set_ghost_wireframe(
        &mut self,
        player_index: usize,
        positions: &[(i32, i32, i32)],
        color_rgb: [f32; 3],
    ) {
        if positions.is_empty() {
            if let Some(gpu) = self.player_gpu.get_mut(player_index) {
                gpu.ghost_buffer = None;
                gpu.ghost_vertex_count = 0;
            }
            return;
        }
        let color = [color_rgb[0], color_rgb[1], color_rgb[2], 0.75];
        let mut verts: Vec<WireVertex> = Vec::with_capacity(positions.len() * 12 * 6);
        // Thin edges: the blow-up cage is a 4×4×4 grid of cells whose shared
        // internal edges overlap, so a thinner line keeps it from reading as a
        // heavy lattice (playtest 2026-06-08).
        for &pos in positions {
            verts.extend(build_wireframe_cube_colored_t([pos.0, pos.1, pos.2], color, 0.004));
        }
        let (device, queue) = (&self.device, &self.queue);
        if let Some(gpu) = self.player_gpu.get_mut(player_index) {
            write_dynamic_vbuf(device, queue, &mut gpu.ghost_buffer, "ghost_wire_buffer",
                bytemuck::cast_slice(&verts));
            gpu.ghost_vertex_count = verts.len() as u32;
        }
    }

    /// #8 — set the spawn-proof overlay: red wireframe cubes around the given
    /// surface cells (mirrors `set_ghost_wireframe` into its own buffer so it
    /// coexists with ghost placement + the targeting outline). Empty = clear.
    /// Spec 48 Phase 2 — upload the visible IR beams for a player. `segments`
    /// are world-space endpoint PAIRS in cell-centre coordinates; each becomes
    /// one LineList segment. Empty clears the buffer. Faint red so a beam reads
    /// as an IR tripline without dominating the scene.
    pub fn set_beam_lines(&mut self, player_index: usize, segments: &[([f32; 3], [f32; 3])]) {
        if segments.is_empty() {
            if let Some(gpu) = self.player_gpu.get_mut(player_index) {
                gpu.beam_buffer = None;
                gpu.beam_vertex_count = 0;
            }
            return;
        }
        let color = [1.0, 0.18, 0.14, 0.55];
        // The wire pipeline is a TriangleList — a bare 2-vertex segment never
        // rasterises (it was silently invisible until 2026-09-06). Thicken each
        // beam into the same cross-of-quads the paint aids use.
        let mut verts: Vec<WireVertex> = Vec::with_capacity(segments.len() * 12);
        for (a, b) in segments {
            push_thick_line(&mut verts, *a, *b, color, 0.006);
        }
        let (device, queue) = (&self.device, &self.queue);
        if let Some(gpu) = self.player_gpu.get_mut(player_index) {
            write_dynamic_vbuf(device, queue, &mut gpu.beam_buffer, "beam_wire_buffer",
                bytemuck::cast_slice(&verts));
            gpu.beam_vertex_count = verts.len() as u32;
        }
    }

    /// Skin painter (2026-09-06) — upload the two precision aids drawn over the
    /// blown-up Workshop mannequin, in one buffer:
    ///
    /// - `grid` — the texel gridlines, near-black and thin, so a kid can see
    ///   where one skin pixel ends and the next begins at the ×4 blow-up.
    /// - `footprint` — the outline of the pixels the NEXT click will paint
    ///   (brush size and face clamping included), warm yellow and thicker so it
    ///   reads on top of the grid it shares edges with.
    ///
    /// Both are world-space endpoint pairs from [`crate::skin_grid`]; each is
    /// expanded here into a thin cross of two quads so a line stays visible from
    /// any angle (the wire pipeline is a TriangleList — a bare 2-vertex segment
    /// would not draw). Empty + empty clears the buffer.
    pub fn set_paint_aid_lines(
        &mut self,
        player_index: usize,
        grid: &[([f32; 3], [f32; 3])],
        footprint: &[([f32; 3], [f32; 3])],
    ) {
        if grid.is_empty() && footprint.is_empty() {
            if let Some(gpu) = self.player_gpu.get_mut(player_index) {
                gpu.paint_aid_buffer = None;
                gpu.paint_aid_vertex_count = 0;
            }
            return;
        }
        // Near-black at low alpha: a graph-paper hint over the skin, never a
        // cage that competes with the colours being painted.
        const GRID_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 0.35];
        const GRID_T: f32 = 0.003;
        // Warm yellow, nearly opaque: this is the "your click lands HERE" cue.
        const FOOTPRINT_COLOR: [f32; 4] = [1.0, 0.95, 0.2, 0.95];
        const FOOTPRINT_T: f32 = 0.008;
        let mut verts: Vec<WireVertex> =
            Vec::with_capacity((grid.len() + footprint.len()) * 12);
        for (a, b) in grid {
            push_thick_line(&mut verts, *a, *b, GRID_COLOR, GRID_T);
        }
        for (a, b) in footprint {
            push_thick_line(&mut verts, *a, *b, FOOTPRINT_COLOR, FOOTPRINT_T);
        }
        let (device, queue) = (&self.device, &self.queue);
        if let Some(gpu) = self.player_gpu.get_mut(player_index) {
            write_dynamic_vbuf(
                device,
                queue,
                &mut gpu.paint_aid_buffer,
                "paint_aid_wire_buffer",
                bytemuck::cast_slice(&verts),
            );
            gpu.paint_aid_vertex_count = verts.len() as u32;
        }
    }

    pub fn set_spawn_markers(&mut self, player_index: usize, positions: &[(i32, i32, i32)]) {
        if positions.is_empty() {
            if let Some(gpu) = self.player_gpu.get_mut(player_index) {
                gpu.spawn_marker_buffer = None;
                gpu.spawn_marker_vertex_count = 0;
            }
            return;
        }
        let color = [0.95, 0.20, 0.20, 0.85]; // red, mostly opaque
        let mut verts: Vec<WireVertex> = Vec::with_capacity(positions.len() * 12 * 6);
        for &pos in positions {
            verts.extend(build_wireframe_cube_colored_t([pos.0, pos.1, pos.2], color, 0.012));
        }
        let (device, queue) = (&self.device, &self.queue);
        if let Some(gpu) = self.player_gpu.get_mut(player_index) {
            write_dynamic_vbuf(
                device,
                queue,
                &mut gpu.spawn_marker_buffer,
                "spawn_marker_buffer",
                bytemuck::cast_slice(&verts),
            );
            gpu.spawn_marker_vertex_count = verts.len() as u32;
        }
    }

    /// #9 — set the build-guide ghosts: per-cell wireframe cubes, each with its
    /// own status colour (green=correct, white=missing, red=wrong). Empty = clear.
    pub fn set_build_guide_markers(
        &mut self,
        player_index: usize,
        cells: &[((i32, i32, i32), [f32; 4])],
    ) {
        if cells.is_empty() {
            if let Some(gpu) = self.player_gpu.get_mut(player_index) {
                gpu.build_guide_buffer = None;
                gpu.build_guide_vertex_count = 0;
            }
            return;
        }
        let mut verts: Vec<WireVertex> = Vec::with_capacity(cells.len() * 12 * 6);
        for (pos, color) in cells {
            verts.extend(build_wireframe_cube_colored_t([pos.0, pos.1, pos.2], *color, 0.01));
        }
        let (device, queue) = (&self.device, &self.queue);
        if let Some(gpu) = self.player_gpu.get_mut(player_index) {
            write_dynamic_vbuf(
                device,
                queue,
                &mut gpu.build_guide_buffer,
                "build_guide_buffer",
                bytemuck::cast_slice(&verts),
            );
            gpu.build_guide_vertex_count = verts.len() as u32;
        }
    }

    /// Trials — set the chase-ghost: translucent wireframe boxes (a runner
    /// silhouette) at the replayed best-run transform. Empty = clear.
    pub fn set_trial_ghost(
        &mut self,
        player_index: usize,
        boxes: &[([f32; 3], [f32; 3], [f32; 4])],
    ) {
        if boxes.is_empty() {
            if let Some(gpu) = self.player_gpu.get_mut(player_index) {
                gpu.trial_ghost_buffer = None;
                gpu.trial_ghost_vertex_count = 0;
            }
            return;
        }
        let mut verts: Vec<WireVertex> = Vec::with_capacity(boxes.len() * 12 * 6);
        for (min, size, color) in boxes {
            verts.extend(build_wireframe_box_t(*min, *size, *color, 0.02));
        }
        let (device, queue) = (&self.device, &self.queue);
        if let Some(gpu) = self.player_gpu.get_mut(player_index) {
            write_dynamic_vbuf(
                device,
                queue,
                &mut gpu.trial_ghost_buffer,
                "trial_ghost_buffer",
                bytemuck::cast_slice(&verts),
            );
            gpu.trial_ghost_vertex_count = verts.len() as u32;
        }
    }

    pub fn set_block_highlight(
        &mut self,
        player_index: usize,
        block_pos: Option<[i32; 3]>,
        thin: bool,
    ) {
        match block_pos {
            Some(pos) => {
                // Track/cable are a shallow floor slab — the outline hugs the
                // rail (0.15 tall) instead of caging a full 1×1 block.
                let verts = if thin {
                    build_wireframe_box_t(
                        [pos[0] as f32, pos[1] as f32, pos[2] as f32],
                        [1.0, 0.15, 1.0],
                        [0.1, 0.1, 0.1, 0.6],
                        0.01,
                    )
                } else {
                    build_wireframe_cube(pos)
                };
                let (device, queue) = (&self.device, &self.queue);
                if let Some(gpu) = self.player_gpu.get_mut(player_index) {
                    write_dynamic_vbuf(device, queue, &mut gpu.wire_buffer, "wire_buffer",
                        bytemuck::cast_slice(&verts));
                    gpu.wire_vertex_count = verts.len() as u32;
                }
            }
            None => {
                if let Some(gpu) = self.player_gpu.get_mut(player_index) {
                    gpu.wire_buffer = None;
                    gpu.wire_vertex_count = 0;
                }
            }
        }
    }

    /// Spec 05 §2.2 — set (or clear) the block-break crack overlay for a player.
    /// `Some((pos, stage))` draws crack-stage `stage` (0-9) over the block at
    /// `pos`; `None` clears it. Mirrors `set_block_highlight`'s buffer handling.
    pub fn set_crack_overlay(
        &mut self,
        player_index: usize,
        overlay: Option<([i32; 3], u8)>,
    ) {
        match overlay {
            Some((pos, stage)) => {
                let tex_layer = crate::block::TEX_CRACK_BASE + stage as u32;
                let verts = build_crack_cube(pos, tex_layer);
                let (device, queue) = (&self.device, &self.queue);
                if let Some(gpu) = self.player_gpu.get_mut(player_index) {
                    write_dynamic_vbuf(device, queue, &mut gpu.crack_buffer, "crack_buffer",
                        bytemuck::cast_slice(&verts));
                    gpu.crack_vertex_count = verts.len() as u32;
                }
            }
            None => {
                if let Some(gpu) = self.player_gpu.get_mut(player_index) {
                    gpu.crack_buffer = None;
                    gpu.crack_vertex_count = 0;
                }
            }
        }
    }

    /// Upload entity cube vertices for this frame.
    pub fn upload_entity_vertices(&mut self, player_index: usize, vertices: &[crate::mesh::Vertex]) {
        if vertices.is_empty() {
            if let Some(gpu) = self.player_gpu.get_mut(player_index) {
                gpu.entity_buffer = None;
                gpu.entity_vertex_count = 0;
            }
            return;
        }
        let (device, queue) = (&self.device, &self.queue);
        if let Some(gpu) = self.player_gpu.get_mut(player_index) {
            write_dynamic_vbuf(device, queue, &mut gpu.entity_buffer, "entity_vertices",
                bytemuck::cast_slice(vertices));
            gpu.entity_vertex_count = vertices.len() as u32;
        }
    }

    /// P1-T4 — upload one viewport's remote-player avatar skin vertices
    /// (box-unwrapped onto the 64x64 skin atlas). Mirrors
    /// [`Self::upload_entity_vertices`]; an empty slice clears the buffer (so a
    /// viewport with no visible remote players draws no avatars).
    pub fn upload_avatar_vertices(&mut self, player_index: usize, vertices: &[crate::mesh::Vertex]) {
        if vertices.is_empty() {
            if let Some(gpu) = self.player_gpu.get_mut(player_index) {
                gpu.avatar_buffer = None;
                gpu.avatar_vertex_count = 0;
            }
            return;
        }
        let (device, queue) = (&self.device, &self.queue);
        if let Some(gpu) = self.player_gpu.get_mut(player_index) {
            write_dynamic_vbuf(device, queue, &mut gpu.avatar_buffer, "avatar_vertices",
                bytemuck::cast_slice(vertices));
            gpu.avatar_vertex_count = vertices.len() as u32;
        }
    }

    /// Write a 64×64 RGBA skin into array `layer` (0 = local player's own skin).
    pub fn write_avatar_skin_layer(&self, layer: u32, rgba: &[u8]) {
        debug_assert_eq!(rgba.len(), 64 * 64 * 4, "avatar skin must be 64x64 RGBA");
        if rgba.len() != 64 * 64 * 4 || layer >= SKIN_LAYERS { return; }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.skin_texture, mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(64 * 4), rows_per_image: Some(64) },
            wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 },
        );
    }
    /// Back-compat (Phase 2): the local player's skin lives in layer 0.
    /// Updates BOTH the 3rd-person avatar and the 1st-person hand (they share
    /// `skin_texture` + `skin_bind_group`). Pixels come from
    /// `cosmetics::decode_skin_64` or `CosmeticDescriptor::skin_rgba`.
    pub fn write_avatar_skin(&self, rgba: &[u8]) { self.write_avatar_skin_layer(0, rgba); }

    /// Render (or return cached) a small front-facing 3D avatar thumbnail for a
    /// wardrobe entry, returning the egui TextureId to display in the grid. The
    /// render is skipped on a cache hit (`key` unchanged since last render), so
    /// steady-state cost is just an `egui::Image`. `key` is the skin's
    /// `skin_key()` — pass it so an edited skin re-renders automatically.
    pub fn render_skin_thumbnail(
        &mut self,
        id: crate::skin_wardrobe::SkinId,
        skin_rgba: &[u8],
        key: u64,
        registry: &crate::block::BlockRegistry,
        arm: crate::skin_uv::ArmModel,
    ) -> egui::TextureId {
        const W: u32 = 96;
        const H: u32 = 144;
        const THUMB_YAW: f32 = std::f32::consts::PI; // face the camera (face on −Z)

        // Fast path: cache hit → no GPU work.
        if let Some(t) = self.skin_thumbs.get(&id)
            && !thumb_needs_render(Some(t.cached_key), key)
            && t.cached_arm == arm {
                return t.egui_id;
            }

        // Lazily create the target (colour + depth + camera + egui registration).
        if !self.skin_thumbs.contains_key(&id) {
            let color = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("skin_thumb_color"),
                size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
            let depth_view = create_depth_texture(&self.device, W, H);
            let (camera_buffer, camera_bind_group) =
                make_viewmodel_camera(&self.device, &self.camera_bind_group_layout);
            let egui_id = self.egui.register_native_texture(
                &self.device, &color_view, wgpu::FilterMode::Nearest,
            );
            self.skin_thumbs.insert(id, SkinThumb {
                color_view, depth_view, camera_buffer, camera_bind_group,
                vbuf: None, vcount: 0, egui_id, cached_key: u64::MAX,
                cached_arm: crate::skin_uv::ArmModel::Classic,
            });
        }

        // Stage the entry's pixels in the reserved preview layer (not layer 0).
        self.write_avatar_skin_layer(SKIN_PREVIEW_LAYER, skin_rgba);

        let ps = crate::protocol::PlayerState {
            player_index: 0, x: 0.0, y: 0.0, z: 0.0, yaw: 0.0, pitch: 0.0,
            health: 20.0, held_kind: 0, held_id: 0, anim_state: 0, flags: 0, skin_key: 0,
        };
        let (skin_verts, _held) =
            crate::entity_model::build_player_avatar_vertices(&ps, 0.0, 0, SKIN_PREVIEW_LAYER, 1.0, registry, arm);
        let view_proj =
            skin_preview_view_proj(THUMB_YAW, PREVIEW_DEFAULT_PITCH, PREVIEW_DEFAULT_DIST, W as f32 / H as f32);
        let eye = skin_preview_eye(THUMB_YAW, PREVIEW_DEFAULT_PITCH, PREVIEW_DEFAULT_DIST);
        let uniform = CameraUniform {
            view_proj: view_proj.to_cols_array_2d(),
            camera_pos: [eye.x, eye.y, eye.z, 1.0],
            sun_dir: [0.3, 1.0, 0.5, 1.0],
            fog: crate::camera::fog_disabled(),
            params: [1.0, 0.0, 0.0, 0.0],
        cam_right: [1.0, 0.0, 0.0, 0.0],
        cam_up: [0.0, 1.0, 0.0, 0.0],
        };

        let device = &self.device;
        let queue = &self.queue;
        let avatar_pipeline = &self.avatar_pipeline;
        let texture_bg = &self.texture_bind_group;
        let skin_bg = &self.skin_bind_group;
        let thumb = self.skin_thumbs.get_mut(&id).expect("inserted above");

        queue.write_buffer(&thumb.camera_buffer, 0, bytemuck::cast_slice(&[uniform]));
        write_dynamic_vbuf(device, queue, &mut thumb.vbuf, "skin_thumb_vbuf",
            bytemuck::cast_slice(&skin_verts));
        thumb.vcount = skin_verts.len() as u32;

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("skin_thumb_encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("skin_thumb_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
                    view: &thumb.color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.06, g: 0.07, b: 0.10, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &thumb.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let (Some(vbuf), true) = (thumb.vbuf.as_ref(), thumb.vcount > 0) {
                pass.set_pipeline(avatar_pipeline);
                pass.set_bind_group(0, &thumb.camera_bind_group, &[]);
                pass.set_bind_group(1, texture_bg, &[]);
                pass.set_bind_group(2, skin_bg, &[]);
                pass.set_vertex_buffer(0, vbuf.slice(..));
                pass.draw(0..thumb.vcount, 0..1);
            }
        }
        queue.submit(std::iter::once(encoder.finish()));
        thumb.cached_key = key;
        thumb.cached_arm = arm;
        thumb.egui_id
    }

    /// Drop a wardrobe entry's cached thumbnail (call when the entry is deleted)
    /// and free its egui texture so it can't leak across deletes.
    pub fn forget_skin_thumbnail(&mut self, id: crate::skin_wardrobe::SkinId) {
        if let Some(t) = self.skin_thumbs.remove(&id) {
            self.egui.free_native_texture(t.egui_id);
        }
    }

    /// Phase 7 — upload the local player's first-person viewmodel vertices
    /// (already in view space). Empty slice clears the buffer.
    pub fn upload_viewmodel_vertices(&mut self, player_index: usize, vertices: &[crate::mesh::Vertex]) {
        if vertices.is_empty() {
            if let Some(gpu) = self.player_gpu.get_mut(player_index) {
                gpu.viewmodel_buffer = None;
                gpu.viewmodel_vertex_count = 0;
            }
            return;
        }
        let (device, queue) = (&self.device, &self.queue);
        if let Some(gpu) = self.player_gpu.get_mut(player_index) {
            write_dynamic_vbuf(device, queue, &mut gpu.viewmodel_buffer, "viewmodel_vertices",
                bytemuck::cast_slice(vertices));
            gpu.viewmodel_vertex_count = vertices.len() as u32;
        }
    }

    /// P1-T5 — upload the first-person ARM mesh (skin-atlas geometry) for a
    /// player. Mirror of [`upload_viewmodel_vertices`] but feeds the buffer the
    /// viewmodel pass binds to `avatar_pipeline` + `skin_bind_group`. Empty
    /// vertices clear the buffer (e.g. when the viewmodel is hidden).
    pub fn upload_viewmodel_skin_vertices(&mut self, player_index: usize, vertices: &[crate::mesh::Vertex]) {
        if vertices.is_empty() {
            if let Some(gpu) = self.player_gpu.get_mut(player_index) {
                gpu.viewmodel_skin_buffer = None;
                gpu.viewmodel_skin_vertex_count = 0;
            }
            return;
        }
        let (device, queue) = (&self.device, &self.queue);
        if let Some(gpu) = self.player_gpu.get_mut(player_index) {
            write_dynamic_vbuf(device, queue, &mut gpu.viewmodel_skin_buffer, "viewmodel_skin_vertices",
                bytemuck::cast_slice(vertices));
            gpu.viewmodel_skin_vertex_count = vertices.len() as u32;
        }
    }

    /// Phase 7 — write the viewmodel camera uniform for a player: view =
    /// identity (geometry is already in view space), projection = a narrow-FOV
    /// perspective with the given viewport aspect, so the held tool keeps a
    /// stable on-screen size regardless of split-screen layout.
    pub fn update_viewmodel_camera(&self, player_index: usize, aspect: f32) {
        if let Some(gpu) = self.player_gpu.get(player_index) {
            let proj = glam::camera::rh::proj::directx::perspective(
                crate::viewmodel::VIEWMODEL_FOV_Y.to_radians(),
                aspect.max(0.0001),
                0.01,
                10.0,
            );
            let uniform = CameraUniform {
                view_proj: proj.to_cols_array_2d(),
                camera_pos: [0.0, 0.0, 0.0, 0.0],
                sun_dir: [0.3, 1.0, 0.5, 1.0],
                fog: crate::camera::default_fog(),
                params: [1.0, 0.0, 0.0, 0.0],
        cam_right: [1.0, 0.0, 0.0, 0.0],
        cam_up: [0.0, 1.0, 0.0, 0.0],
            };
            self.queue.write_buffer(
                &gpu.viewmodel_camera_buffer,
                0,
                bytemuck::cast_slice(&[uniform]),
            );
        }
    }

    pub fn render(&mut self, window: &std::sync::Arc<winit::window::Window>, screens: &[crate::screen::Screen]) -> Result<(), SurfaceError> {
        // Advance any animated textures (Spec 03 §3.4) for this frame before the
        // scene draws, using the game-tick clock set by `set_anim_clock`.
        self.advance_animated_textures();
        self.ensure_surface(window);
        // MUST NOT panic when the surface is missing. On Android the native
        // window is destroyed while backgrounded, `App::suspended` drops the
        // surface, and `ensure_surface` legitimately cannot rebuild until the
        // window returns — an `.expect()` here killed the event-loop thread on
        // a Pixel 8 and looked like a permanent hang. `Transient` is the
        // existing "skip this frame, retry next" contract.
        let Some(surface) = self.surface.as_ref() else {
            return Err(SurfaceError::Transient);
        };
        let output = acquire_surface_frame(surface)?;
        // Render through the sRGB format (a view alias of the non-sRGB canvas on
        // WebGPU; identical to the texture format on native) so linear shader
        // output is gamma-encoded on write — otherwise the scene is too dark.
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.format),
            ..Default::default()
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render_encoder"),
            });

        // Ensure a cached crosshair buffer exists for every viewport size in use
        // (Spec 39 A2). Done first (this mutates the cache) so the draw block
        // below holds only immutable borrows of self.
        for screen in screens {
            let key = (screen.viewport.width, screen.viewport.height);
            if !self.crosshair_cache.contains_key(&key) {
                let buf = create_crosshair_buffer(&self.device, key.0, key.1);
                self.crosshair_cache.insert(key, buf);
            }
        }

        let mut frame_stats = DrawStats::default();
        {
            // Render scale (Spec 39 P5): when scaled, the world draws into the
            // smaller offscreen target and is upscale-blitted to the surface
            // afterwards; otherwise it draws straight to the surface. The UI
            // (egui) always draws at native resolution, after this block.
            let scaled = self.offscreen_color_view.is_some();
            let (world_color, world_depth) = if scaled {
                (
                    self.offscreen_color_view.as_ref().unwrap(),
                    self.offscreen_depth_view.as_ref().unwrap(),
                )
            } else {
                (&view, &self.depth_texture_view)
            };
            let s = if scaled { self.render_scale } else { 1.0 };

            // Clear the world target (color + depth).
            {
                let _clear_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("clear_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                        view: world_color,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: self.sky_color[0] as f64,
                                g: self.sky_color[1] as f64,
                                b: self.sky_color[2] as f64,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: world_depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
            }

            for screen in screens.iter() {
                let crate::screen::ScreenContent::LocalPlayer(player_index) = screen.content;
                let crosshair_buf =
                    &self.crosshair_cache[&(screen.viewport.width, screen.viewport.height)];
                if let Some(gpu) = self.player_gpu.get(player_index) {
                    // Build this player's culling frustum from the cached view-proj (A1).
                    let frustum = crate::camera::Frustum::from_view_proj(gpu.last_view_proj);
                    // Scale the viewport rect into the (possibly smaller) target's
                    // pixel space. At scale 1.0 this is identical to screen.viewport.
                    let vp = crate::screen::ViewportRect {
                        x: ((screen.viewport.x as f32) * s) as u32,
                        y: ((screen.viewport.y as f32) * s) as u32,
                        width: ((screen.viewport.width as f32) * s).round().max(1.0) as u32,
                        height: ((screen.viewport.height as f32) * s).round().max(1.0) as u32,
                    };
                    let vp_stats = render_world_viewport(
                        &self.device,
                        &mut encoder,
                        world_color,
                        world_depth,
                        gpu,
                        &vp,
                        self.sky_color,
                        false, // never clear — already cleared above
                        &self.render_pipeline,
                        &self.water_pipeline,
                        &self.transparent_pipeline,
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
                        &self.painting_pipeline,
                        &self.paintings,
                        &self.painting_textures,
                        &self.painting_alpha_pipeline,
                        &self.billboards,
                        &frustum,
                        crosshair_buf,
                        // Cinematic Director hide-HUD suppresses only slot 0's
                        // crosshair (its detached viewport); other split-screen
                        // viewports keep theirs.
                        !(self.hide_crosshair && player_index == 0),
                    );
                    frame_stats.draw_calls += vp_stats.draw_calls;
                    frame_stats.culled += vp_stats.culled;
                }
            }

            // Upscale-blit the offscreen world onto the surface (Spec 39 P5).
            // Only present when scaled; the fullscreen triangle covers every pixel.
            if let Some(bind_group) = self.offscreen_bind_group.as_ref() {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("blit_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    ..Default::default()
                });
                pass.set_pipeline(&self.blit_pipeline);
                pass.set_bind_group(0, bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        self.last_draw_stats = frame_stats;

        // --- egui pass (full screen, all UI: hotbar, hearts, menus, crafting, debug) ---
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.width, self.height],
            pixels_per_point: self.egui.ctx.pixels_per_point(),
        };
        self.egui.end_frame(
            &self.device,
            &self.queue,
            &mut encoder,
            &view,
            screen_descriptor,
            window,
        );

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }

    /// The texture-array layer ceiling this device granted. The adopt guard
    /// (`apply_adopted_override_bytes`) rejects any adopted override-set whose
    /// rebuilt layer count would exceed this, so a malicious/huge shared blob
    /// can never push `rebuild_block_textures` past the device limit. Consumed
    /// only on the wasm adopt path (+ tests) — hence the native dead-code allow.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn max_texture_array_layers(&self) -> u32 {
        self.device.limits().max_texture_array_layers
    }

    /// Spec 39 A6 — turn block-atlas mipmaps + mip filtering on or off live, the
    /// same way the texture-pack picker hot-swaps the atlas: recreate the sampler,
    /// rebuild the array (now with or without a mip chain) and rebind. No restart,
    /// no re-mesh — vertex data is untouched, so the emissive `light` sentinel and
    /// the per-face tint layers (both per-vertex, not per-texture) are unaffected.
    ///
    /// `appended` is the override registry's appended layers, exactly as passed to
    /// [`Renderer::rebuild_block_textures`], so a Workshop reskin survives the
    /// toggle. A no-op when the dial already matches.
    pub fn set_mipmaps(&mut self, on: bool, appended: &[Vec<u8>]) {
        if self.block_mipmaps == on {
            return;
        }
        self.block_mipmaps = on;
        self.block_sampler = create_block_sampler(&self.device, on);
        self.rebuild_block_textures(appended);
        log::info!("block-atlas mipmaps {}", if on { "on" } else { "off" });
    }

    /// Spec 40 (The Workshop) §3.2 — runtime texture injection. Rebuild the block
    /// texture array so it holds the stock layers **plus** the authored override
    /// layers, then recreate the view + `texture_bind_group` the chunk/entity
    /// pipelines sample. `appended` is `OverrideRegistry::appended_layers()` — each
    /// entry a 16×16 RGBA buffer uploaded at array layer `texture_count() + i`,
    /// matching the indices the mesher/entity-builder emit.
    ///
    /// Called whenever the override set changes (Phase B world-load with saved
    /// overrides; Phase C pin/paint apply). With `appended` empty this restores the
    /// stock array exactly (no override layers), so an override-free game is
    /// byte-identical. Callers must also re-mesh loaded chunks so block faces pick
    /// up the new layers (entity faces resolve per-build, no re-mesh needed).
    /// Returns `true` when the rebuild was applied, `false` when it was
    /// **refused** because `appended` would push the atlas past the device's
    /// `max_texture_array_layers` — in which case nothing is touched and the
    /// caller keeps whatever atlas is currently live. This is the single
    /// choke point every caller routes through (Workshop pin/reset, Stash
    /// wardrobe load, and Wardrobe "Set active"), so the layer-cap guard here
    /// covers all of them at once (P6 audit: "Wardrobe 'Set active' rebuilds
    /// the block atlas with no layer-cap check" — the pin/adopt guards
    /// upstream don't cover activating an already-adopted design later).
    pub fn rebuild_block_textures(&mut self, appended: &[Vec<u8>]) -> bool {
        // Pack-overridden stock layers (P2) so Workshop reskins sit atop the
        // active pack, not the raw procedural set.
        let base_textures = crate::texture_registry::base_textures();
        // Atlas resolution follows the (possibly hi-res) base; scale the Workshop's
        // 16×16 appended override layers to match so the array stays uniform (§11.3).
        let tex_size = base_textures
            .first()
            .map_or(16, |t| crate::texture_registry::square_side(t.len()));
        let appended: Vec<Vec<u8>> = appended
            .iter()
            .map(|a| {
                crate::texture_registry::scale_layer(
                    a,
                    crate::texture_registry::square_side(a.len()),
                    tex_size,
                )
            })
            .collect();
        let total = base_textures.len() as u32 + appended.len() as u32;
        let granted = self.device.limits().max_texture_array_layers;
        if total > granted {
            // Refuse BEFORE any GPU call or `self.*` mutation, so the
            // currently-live atlas (and everything that binds it) is
            // untouched — no `create_texture` validation panic, no
            // half-applied state.
            log::error!(
                "rebuild_block_textures: refusing — {total} layers needed but this device \
                 granted only {granted} (max_texture_array_layers); keeping the current atlas"
            );
            return false;
        }
        assert_block_texture_layers_fit(total, &self.device);

        // Stock layers first (0..texture_count()), then the appended override
        // layers — the exact order `OverrideRegistry` assigned them. Mip levels
        // follow the live `block_mipmaps` dial (Spec 39 A6).
        let block_texture = create_block_texture_array(
            &self.device,
            &self.queue,
            base_textures.iter().chain(appended.iter()),
            tex_size,
            total,
            self.block_mipmaps,
        );

        let block_texture_view = block_texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        // The view keeps the GPU texture alive (wgpu ref-counts internally), so the
        // local `block_texture` handle can drop — same pattern as the constructors.
        self.texture_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("texture_bind_group"),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&block_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.block_sampler),
                },
            ],
        });

        // Retain the new array + refresh the active pack's animated textures so
        // per-frame animation targets the live atlas at its current resolution.
        self.block_texture = block_texture;
        self.block_tex_size = tex_size;
        self.animated_textures = Self::load_active_animations(tex_size);
        self.animated_last_frame = vec![u32::MAX; self.animated_textures.len()];
        true
    }

    /// Load the active pack's animated textures at the atlas resolution `tex_size`.
    /// Native reads the active pack dir (Spec 03 §3.4); WASM has no filesystem, so
    /// these are fed by the fetched pack in P4 — empty until then.
    fn load_active_animations(tex_size: u32) -> Vec<crate::texture_anim::AnimatedTexture> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::texture_registry::active_pack_dir()
                .map(|d| crate::texture_registry::collect_pack_animations(&d, tex_size))
                .unwrap_or_default()
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = tex_size;
            Vec::new()
        }
    }

    /// Set the game-tick clock that drives texture animation (call once per frame
    /// from the world `tick_counter` so animation pauses with the game).
    pub fn set_anim_clock(&mut self, tick: u64) {
        self.anim_clock = tick;
    }

    /// Re-upload the current frame of every animated texture whose frame changed
    /// since the last call — a single-layer `write_texture` each, not a full atlas
    /// rebuild (Spec 03 §3.4). A no-op when the active pack has no animations.
    pub fn advance_animated_textures(&mut self) {
        if self.animated_textures.is_empty() {
            return;
        }
        let tick = self.anim_clock;
        let size = self.block_tex_size;
        let frame_bytes = (size * size * 4) as usize;
        for (i, anim) in self.animated_textures.iter().enumerate() {
            let frame = anim.frame_index_at(tick);
            if self.animated_last_frame.get(i).copied() == Some(frame) {
                continue; // already showing this frame
            }
            let pixels = anim.pixels_at(tick);
            if pixels.len() == frame_bytes {
                // Spec 39 A6 — with mipmaps on, this layer's whole chain has to
                // move with the frame or distant animated blocks freeze on the
                // first frame they were uploaded with.
                crate::mipmap::write_layer_with_mips(
                    &self.queue,
                    &self.block_texture,
                    anim.layer_index,
                    pixels,
                    size,
                    crate::mipmap::atlas_mip_levels(size, self.block_mipmaps),
                );
            }
            if let Some(slot) = self.animated_last_frame.get_mut(i) {
                *slot = frame;
            }
        }
    }

    /// Create a headless renderer (no window, no surface).
    pub async fn new_headless(width: u32, height: u32, textures: &[Vec<u8>]) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });

        // Prefer a real GPU: the software fallback (llvmpipe) caps
        // `max_texture_array_layers` at 256, which can't hold the block-texture
        // array (~506 layers) and panics. Fall back to software only if no real
        // adapter exists (e.g. CI with no GPU) — that path won't build the big
        // array in practice (menu-only shots), but real hardware is the norm.
        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
        {
            Ok(a) => a,
            Err(_) => instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: None,
                    force_fallback_adapter: true,
                })
                .await
                .expect("Failed to find a suitable GPU adapter (headless)"),
        };

        log::info!("Headless adapter: {:?}", adapter.get_info().name);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("axenstax_headless_device"),
                required_features: wgpu::Features::empty(),
                // Match the windowed path: the block-texture array needs more than
                // the default 256 `max_texture_array_layers`, so lift it to the
                // adapter's limit (else create_texture('block_textures') panics).
                required_limits: wgpu::Limits {
                    max_texture_array_layers: adapter.limits().max_texture_array_layers,
                    ..wgpu::Limits::default()
                },
                ..Default::default()
            })
            .await
            .expect("Failed to create GPU device");

        let format = wgpu::TextureFormat::Rgba8UnormSrgb;

        let color_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("headless_color_target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let depth_texture_view = create_depth_texture(&device, width, height);

        // --- Camera uniform ---
        let camera_uniform = crate::camera::CameraUniform {
            view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
            camera_pos: [0.0, 0.0, 0.0, 0.0],
            sun_dir: [0.3, 1.0, 0.5, 1.0],
            fog: crate::camera::default_fog(),
            params: [1.0, 0.0, 0.0, 0.0],
        cam_right: [1.0, 0.0, 0.0, 0.0],
        cam_up: [0.0, 1.0, 0.0, 0.0],
        };
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera_uniform"),
            contents: bytemuck::cast_slice(&[camera_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("camera_bind_group_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera_bind_group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        // --- Block texture array ---
        // Atlas resolution is inferred from the layer bytes (Spec 03 §11.3): a
        // resolution-agnostic pack hands us pre-scaled res×res layers.
        let tex_size = textures
            .first()
            .map_or(16, |t| crate::texture_registry::square_side(t.len()));
        let num_layers = texture_gen::texture_count();
        assert_block_texture_layers_fit(num_layers, &device);

        // Mipmaps start OFF (today's look); `set_mipmaps` rebuilds the array +
        // sampler live when the Graphics dial turns them on (Spec 39 A6).
        let block_mipmaps = false;
        let block_texture = create_block_texture_array(
            &device,
            &queue,
            textures.iter(),
            tex_size,
            num_layers,
            block_mipmaps,
        );

        let block_texture_view = block_texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let block_sampler = create_block_sampler(&device, block_mipmaps);

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("texture_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("texture_bind_group"),
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&block_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&block_sampler),
                },
            ],
        });

        // --- Pipelines (same as windowed, using fixed format) ---
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("chunk_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("chunk_pipeline_layout"),
            bind_group_layouts: &[Some(&camera_bind_group_layout), Some(&texture_bind_group_layout)],
            immediate_size: 0,
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("chunk_render_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Vertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // --- Entity pipeline (same as chunk but NO back-face culling) ---
        let entity_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("entity_render_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Vertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // --- Avatar pipeline (64×64 skin texture, group 2) — same as the
        // windowed path, targeting the offscreen `format`. ---
        let AvatarResources {
            skin_texture,
            skin_bind_group_layout,
            skin_bind_group,
            avatar_pipeline,
        } = create_avatar_resources(
            &device,
            &queue,
            format,
            &block_sampler,
            &camera_bind_group_layout,
            &texture_bind_group_layout,
        );

        let (
            painting_pipeline,
            painting_alpha_pipeline,
            painting_bind_group_layout,
            painting_sampler,
        ) = create_painting_resources(
                &device,
                &shader,
                format,
                &camera_bind_group_layout,
                &texture_bind_group_layout,
            );

        // --- Water pipeline (alpha blended, depth read-only) ---
        let water_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water_render_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Vertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_water"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // Water visible from both sides (swimming underwater)
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // #131 — transparent-solid pipeline (glass, …); mirrors water + fs_transparent.
        let transparent_pipeline =
            build_transparent_pipeline(&device, &shader, &pipeline_layout, format);

        // --- egui integration (headless — for consistency, though screenshots don't need interactive UI) ---
        // BRIDGE: headless egui init uses a dummy window. For screenshot mode,
        // egui is unused but the struct field must be populated.
        // Replace when headless gets its own Renderer variant.
        let egui = EguiIntegration::new_headless(&device, format, textures);

        let overlay_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("overlay_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("overlay.wgsl").into()),
        });

        let crosshair_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("crosshair_pipeline_layout"),
                bind_group_layouts: &[],
                immediate_size: 0,
            });

        let crosshair_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("crosshair_pipeline"),
                layout: Some(&crosshair_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &overlay_shader,
                    entry_point: Some("vs_crosshair"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<CrosshairVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &overlay_shader,
                    entry_point: Some("fs_crosshair"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        let crosshair_buffer = create_crosshair_buffer(&device, width, height);

        let wire_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("wire_pipeline_layout"),
                bind_group_layouts: &[Some(&camera_bind_group_layout)],
                immediate_size: 0,
            });

        let wire_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("wire_pipeline"),
                layout: Some(&wire_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &overlay_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<WireVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &overlay_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState {
                        constant: -2,
                        slope_scale: -1.0,
                        clamp: 0.0,
                    },
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        // Active pack's animated textures (Spec 03 §3.4); empty for the default pack.
        let animated_textures = Self::load_active_animations(tex_size);

        let crack_pipeline = make_overlay_pipeline(
            &device, &shader, &pipeline_layout, format, "fs_crack", "crack_render_pipeline",
        );
        let decal_pipeline = make_overlay_pipeline(
            &device, &shader, &pipeline_layout, format, "fs_decal", "decal_render_pipeline",
        );

        let plant_pipeline = create_plant_pipeline(&device, &pipeline_layout, &shader, format);
        let (particle_pipeline, particle_geo_vbuf, particle_geo_ibuf, particle_geo_index_count, particle_instance_buffer) =
            create_particle_gpu(&device, &pipeline_layout, &shader, format);
        let (plant_geo_vbuf, plant_geo_ibuf, plant_geo_index_count) = create_plant_geo(&device);
        let micro_pipeline = create_micro_pipeline(&device, &pipeline_layout, &shader, format);
        let (blit_pipeline, blit_sampler, blit_bind_group_layout) =
            build_blit_resources(&device, &shader, format);

        let (vm_cam_buf, vm_cam_bg) =
            make_viewmodel_camera(&device, &camera_bind_group_layout);

        Self {
            surface: None,
            instance,
            device,
            queue,
            format,
            // No surface to configure; headless renders straight to the sRGB
            // color target, so config == render format.
            config_format: format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            render_pipeline,
            depth_texture_view,
            camera_bind_group_layout,
            texture_bind_group,
            block_sampler,
            texture_bind_group_layout,
            animated_last_frame: vec![u32::MAX; animated_textures.len()],
            animated_textures,
            block_texture,
            block_tex_size: tex_size,
            block_mipmaps,
            anim_clock: 0,
            chunk_meshes: ahash::AHashMap::new(),
            water_pipeline,
            water_meshes: ahash::AHashMap::new(),
            transparent_pipeline,
            transparent_meshes: ahash::AHashMap::new(),
            plant_pipeline,
            plant_geo_vbuf,
            plant_geo_ibuf,
            plant_geo_index_count,
            plant_meshes: ahash::AHashMap::new(),
            particle_pipeline,
            particle_geo_vbuf,
            particle_geo_ibuf,
            particle_geo_index_count,
            particle_instance_buffer,
            particle_count: 0,
            decal_meshes: ahash::AHashMap::new(),
            micro_pipeline,
            micro_geo: ahash::AHashMap::new(),
            micro_meshes: ahash::AHashMap::new(),
            micro_billboard_meshes: ahash::AHashMap::new(),
            micro_lod_dist: f32::MAX,
            crosshair_pipeline,
            crosshair_buffer,
            crosshair_vertex_count: 12,
            hide_crosshair: false,
            wire_pipeline,
            crack_pipeline,
            decal_pipeline,
            entity_pipeline,
            painting_pipeline,
            painting_alpha_pipeline,
            painting_bind_group_layout,
            painting_sampler,
            painting_textures: ahash::AHashMap::new(),
            paintings: Vec::new(),
            billboards: Vec::new(),
            avatar_pipeline,
            skin_bind_group,
            skin_texture,
            skin_bind_group_layout,
            skin_thumbs: std::collections::HashMap::new(),
            player_gpu: vec![PlayerGpuResources {
                camera_buffer,
                camera_bind_group,
                entity_buffer: None,
                entity_vertex_count: 0,
                avatar_buffer: None,
                avatar_vertex_count: 0,
                wire_buffer: None,
                wire_vertex_count: 0,
                crack_buffer: None,
                crack_vertex_count: 0,
                ghost_buffer: None,
                ghost_vertex_count: 0,
                spawn_marker_buffer: None,
                spawn_marker_vertex_count: 0,
                build_guide_buffer: None,
                build_guide_vertex_count: 0,
                trial_ghost_buffer: None,
                trial_ghost_vertex_count: 0,
                beam_buffer: None,
                beam_vertex_count: 0,
                paint_aid_buffer: None,
                paint_aid_vertex_count: 0,
                viewmodel_buffer: None,
                viewmodel_vertex_count: 0,
                viewmodel_skin_buffer: None,
                viewmodel_skin_vertex_count: 0,
                viewmodel_camera_buffer: vm_cam_buf,
                viewmodel_camera_bind_group: vm_cam_bg,
                last_view_proj: glam::Mat4::IDENTITY,
            }],
            last_draw_stats: DrawStats::default(),
            crosshair_cache: ahash::AHashMap::new(),
            render_scale: 1.0,
            offscreen_color_view: None,
            offscreen_depth_view: None,
            offscreen_bind_group: None,
            blit_pipeline,
            blit_sampler,
            blit_bind_group_layout,
            egui,
            sky_color: [0.53, 0.72, 0.90],
            color_target: Some(color_texture),
        }
    }

    /// Render to the offscreen texture and save as PNG.
    pub fn render_to_png(&self, path: &str) {
        let color_target = self.color_target.as_ref().expect("render_to_png requires headless renderer");
        let view = color_target.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("headless_encoder"),
            });

        // --- Pass 1: World chunks ---
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("headless_world_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: self.sky_color[0] as f64,
                            g: self.sky_color[1] as f64,
                            b: self.sky_color[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_texture_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.player_gpu[0].camera_bind_group, &[]);
            render_pass.set_bind_group(1, &self.texture_bind_group, &[]);

            for gpu_mesh in self.chunk_meshes.values() {
                render_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                render_pass
                    .set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..gpu_mesh.index_count, 0, 0..1);
            }
        }

        // --- Micro-models (owner-inbox #18) — draw baked shells so headless
        // screenshots are faithful to the in-game micro pass (one draw per
        // (chunk, type); no frustum cull needed for a one-off screenshot). ---
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("headless_micro_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_texture_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            render_pass.set_pipeline(&self.micro_pipeline);
            render_pass.set_bind_group(0, &self.player_gpu[0].camera_bind_group, &[]);
            render_pass.set_bind_group(1, &self.texture_bind_group, &[]);
            for batches in self.micro_meshes.values() {
                for (block_id, insts) in batches {
                    if let Some(geo) = self.micro_geo.get(block_id) {
                        render_pass.set_vertex_buffer(0, geo.vertex_buffer.slice(..));
                        render_pass
                            .set_index_buffer(geo.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                        render_pass.set_vertex_buffer(1, insts.buffer.slice(..));
                        render_pass.draw_indexed(0..geo.index_count, 0, 0..insts.count);
                    }
                }
            }
        }

        // --- Entity pass (Spec 40 — Workshop mannequins + any uploaded entities) ---
        if let (Some(entity_buf), count) = (
            self.player_gpu[0].entity_buffer.as_ref(),
            self.player_gpu[0].entity_vertex_count,
        )
            && count > 0 {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("headless_entity_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.depth_texture_view,
                        depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_pipeline(&self.entity_pipeline);
                pass.set_bind_group(0, &self.player_gpu[0].camera_bind_group, &[]);
                pass.set_bind_group(1, &self.texture_bind_group, &[]);
                pass.set_vertex_buffer(0, entity_buf.slice(..));
                pass.draw(0..count, 0..1);
            }

        // --- Avatar pass (third-person self-avatar; 64×64 skin atlas) ---
        // Mirrors the live render's avatar pass so a headless third-person shot
        // (`--shot-3p`) actually shows the player's body. All three bind groups
        // are set even though the avatar shader only samples the skin (group 2),
        // because wgpu validates every declared group.
        if let (Some(avatar_buf), count) = (
            self.player_gpu[0].avatar_buffer.as_ref(),
            self.player_gpu[0].avatar_vertex_count,
        )
            && count > 0 {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("headless_avatar_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.depth_texture_view,
                        depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_pipeline(&self.avatar_pipeline);
                pass.set_bind_group(0, &self.player_gpu[0].camera_bind_group, &[]);
                pass.set_bind_group(1, &self.texture_bind_group, &[]);
                pass.set_bind_group(2, &self.skin_bind_group, &[]);
                pass.set_vertex_buffer(0, avatar_buf.slice(..));
                pass.draw(0..count, 0..1);
            }

        // --- Pass 1.5: Water (alpha blended) ---
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("headless_water_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_texture_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });

            // #131 — solid+transparent blocks (glass, …) in the screenshot path too.
            render_pass.set_pipeline(&self.transparent_pipeline);
            render_pass.set_bind_group(0, &self.player_gpu[0].camera_bind_group, &[]);
            render_pass.set_bind_group(1, &self.texture_bind_group, &[]);
            for gpu_mesh in self.transparent_meshes.values() {
                render_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                render_pass
                    .set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..gpu_mesh.index_count, 0, 0..1);
            }

            render_pass.set_pipeline(&self.water_pipeline);
            render_pass.set_bind_group(0, &self.player_gpu[0].camera_bind_group, &[]);
            render_pass.set_bind_group(1, &self.texture_bind_group, &[]);

            for gpu_mesh in self.water_meshes.values() {
                render_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                render_pass
                    .set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..gpu_mesh.index_count, 0, 0..1);
            }
        }

        // --- Pass 2: Crosshair HUD ---
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("headless_hud_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            render_pass.set_pipeline(&self.crosshair_pipeline);
            render_pass.set_vertex_buffer(0, self.crosshair_buffer.slice(..));
            render_pass.draw(0..self.crosshair_vertex_count, 0..1);
        }

        // --- Copy texture to staging buffer ---
        let bytes_per_row = (self.width * 4 + 255) & !255; // align to 256
        let buffer_size = (bytes_per_row * self.height) as u64;

        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot_staging"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: color_target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        // Map and read pixels
        let buffer_slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().expect("Failed to map staging buffer");

        let data = buffer_slice.get_mapped_range();

        // Remove row padding and collect pixel data
        let mut pixels = Vec::with_capacity((self.width * self.height * 4) as usize);
        for y in 0..self.height {
            let offset = (y * bytes_per_row) as usize;
            pixels.extend_from_slice(&data[offset..offset + (self.width * 4) as usize]);
        }
        drop(data);
        staging.unmap();

        // Save as PNG
        let img: image::RgbaImage =
            image::ImageBuffer::from_raw(self.width, self.height, pixels)
                .expect("Failed to create image buffer");
        img.save(path).expect("Failed to save screenshot");
        log::info!("Screenshot saved to {path}");
    }

    /// Render an egui menu screen (e.g. the lobby) headlessly to a PNG. Clears to
    /// the menu backdrop, runs `run_ui` to build the UI at the renderer's size,
    /// paints it offscreen, and saves. Used by `--shot-lobby` to iterate on menu
    /// UX without opening a window. Requires a headless renderer (`color_target`).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render_menu_to_png(&mut self, path: &str, run_ui: impl FnOnce(&egui::Context)) {
        let color_target = self.color_target.as_ref().expect("render_menu_to_png requires headless renderer");
        let view = color_target.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("headless_menu_encoder"),
        });
        // Clear to the menu backdrop.
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("headless_menu_clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(menu_backdrop_color(self.format)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
        }
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.width, self.height],
            pixels_per_point: 1.0,
        };
        self.egui.run_and_paint_headless(
            &self.device, &self.queue, &mut encoder, &view, screen_descriptor, run_ui,
        );
        self.queue.submit(std::iter::once(encoder.finish()));
        self.save_color_target_png(path);
    }

    /// Copy the headless color target back to the CPU and write it as a PNG.
    #[cfg(not(target_arch = "wasm32"))]
    fn save_color_target_png(&self, path: &str) {
        let color_target = self.color_target.as_ref().expect("save_color_target_png requires headless renderer");
        let bytes_per_row = (self.width * 4 + 255) & !255;
        let buffer_size = (bytes_per_row * self.height) as u64;
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("menu_screenshot_staging"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("menu_readback_encoder"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: color_target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 },
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        let buffer_slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| { tx.send(result).unwrap(); });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().expect("Failed to map menu staging buffer");
        let data = buffer_slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((self.width * self.height * 4) as usize);
        for y in 0..self.height {
            let offset = (y * bytes_per_row) as usize;
            pixels.extend_from_slice(&data[offset..offset + (self.width * 4) as usize]);
        }
        drop(data);
        staging.unmap();
        let img: image::RgbaImage =
            image::ImageBuffer::from_raw(self.width, self.height, pixels)
                .expect("Failed to create menu image buffer");
        img.save(path).expect("Failed to save menu screenshot");
        log::info!("Menu screenshot saved to {path}");
    }

    /// Upload menu vertices for this frame.
    /// Render just the menu screen (no world geometry, but egui renders).
    pub fn render_menu_only(&mut self, window: &std::sync::Arc<winit::window::Window>) -> Result<(), SurfaceError> {
        self.ensure_surface(window);
        // See `render()`: skipping the frame, never panicking, is what keeps the
        // event loop alive while Android has taken the window away.
        let Some(surface) = self.surface.as_ref() else {
            return Err(SurfaceError::Transient);
        };
        let output = acquire_surface_frame(surface)?;
        // sRGB render view alias — see `render()`.
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.format),
            ..Default::default()
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("menu_encoder"),
        });

        // Clear to dark background
        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("menu_clear_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(menu_backdrop_color(self.format)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
        }

        // Render egui (menu UI built earlier this frame)
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.width, self.height],
            pixels_per_point: self.egui.ctx.pixels_per_point(),
        };
        self.egui.end_frame(
            &self.device,
            &self.queue,
            &mut encoder,
            &view,
            screen_descriptor,
            window,
        );

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        Ok(())
    }
}

/// Menu / loading-screen clear colour: Copperline Deep Frontier `#0D1B1E`.
///
/// wgpu clear values are *linear*; on an sRGB render view the hardware encodes
/// them on write, so the brand's sRGB bytes must be decoded first. (The old
/// hand-written `0.05/0.07/0.12` encoded to the slate `#424c63` that showed
/// through the transparent menu panel and the loading screen.)
fn menu_backdrop_color(format: wgpu::TextureFormat) -> wgpu::Color {
    let c = crate::brand::DEEP_FRONTIER;
    let ch = |v: u8| -> f64 {
        let s = f64::from(v) / 255.0;
        if !format.is_srgb() {
            s
        } else if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    wgpu::Color { r: ch(c.r()), g: ch(c.g()), b: ch(c.b()), a: 1.0 }
}

/// Render all world passes (chunks, water, entities, wireframe, crosshair) for a single viewport.
/// Free function to avoid borrow checker issues — render() is &mut self (for egui) but needs
/// to loop over player_gpu while calling this.
fn render_world_viewport(
    _device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    target_view: &wgpu::TextureView,
    depth_view: &wgpu::TextureView,
    gpu: &PlayerGpuResources,
    viewport: &crate::screen::ViewportRect,
    sky_color: [f32; 3],
    clear: bool,
    render_pipeline: &wgpu::RenderPipeline,
    water_pipeline: &wgpu::RenderPipeline,
    transparent_pipeline: &wgpu::RenderPipeline,
    entity_pipeline: &wgpu::RenderPipeline,
    avatar_pipeline: &wgpu::RenderPipeline,
    wire_pipeline: &wgpu::RenderPipeline,
    crack_pipeline: &wgpu::RenderPipeline,
    crosshair_pipeline: &wgpu::RenderPipeline,
    texture_bind_group: &wgpu::BindGroup,
    skin_bind_group: &wgpu::BindGroup,
    chunk_meshes: &ahash::AHashMap<(i32, i32, i32), GpuChunkMesh>,
    water_meshes: &ahash::AHashMap<(i32, i32, i32), GpuChunkMesh>,
    transparent_meshes: &ahash::AHashMap<(i32, i32, i32), GpuChunkMesh>,
    plant_pipeline: &wgpu::RenderPipeline,
    plant_geo_vbuf: &wgpu::Buffer,
    plant_geo_ibuf: &wgpu::Buffer,
    plant_geo_index_count: u32,
    plant_meshes: &ahash::AHashMap<(i32, i32, i32), GpuPlantInstances>,
    particle_pipeline: &wgpu::RenderPipeline,
    particle_geo_vbuf: &wgpu::Buffer,
    particle_geo_ibuf: &wgpu::Buffer,
    particle_geo_index_count: u32,
    particle_instance_buffer: &wgpu::Buffer,
    particle_count: u32,
    decal_pipeline: &wgpu::RenderPipeline,
    decal_meshes: &ahash::AHashMap<(i32, i32, i32), GpuDecalMesh>,
    micro_pipeline: &wgpu::RenderPipeline,
    micro_geo: &ahash::AHashMap<crate::block::BlockId, GpuChunkMesh>,
    micro_meshes: &ahash::AHashMap<(i32, i32, i32), Vec<(crate::block::BlockId, GpuPlantInstances)>>,
    micro_billboard_meshes: &ahash::AHashMap<(i32, i32, i32), GpuPlantInstances>,
    micro_lod_dist: f32,
    painting_pipeline: &wgpu::RenderPipeline,
    paintings: &[PaintingQuad],
    painting_textures: &ahash::AHashMap<u64, PaintingTex>,
    painting_alpha_pipeline: &wgpu::RenderPipeline,
    billboards: &[BillboardQuad],
    frustum: &crate::camera::Frustum,
    crosshair_buf: &wgpu::Buffer,
    show_crosshair: bool,
) -> DrawStats {
    let mut stats = DrawStats::default();
    // Chunk AABB from its (cx, cy, cz) key: 16-block cube at the chunk origin.
    let cs = crate::chunk::CHUNK_SIZE as f32;
    let chunk_aabb = |cx: i32, cy: i32, cz: i32| -> (glam::Vec3, glam::Vec3) {
        let min = glam::Vec3::new(cx as f32 * cs, cy as f32 * cs, cz as f32 * cs);
        (min, min + glam::Vec3::splat(cs))
    };

    let vx = viewport.x as f32;
    let vy = viewport.y as f32;
    let vw = viewport.width as f32;
    let vh = viewport.height as f32;

    let color_load = if clear {
        wgpu::LoadOp::Clear(wgpu::Color {
            r: sky_color[0] as f64,
            g: sky_color[1] as f64,
            b: sky_color[2] as f64,
            a: 1.0,
        })
    } else {
        wgpu::LoadOp::Load
    };
    let depth_load: wgpu::LoadOp<f32> = if clear {
        wgpu::LoadOp::Clear(1.0)
    } else {
        wgpu::LoadOp::Load
    };

    // --- Pass 1: World chunks (textured) ---
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("world_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: color_load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: depth_load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(render_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_bind_group(1, texture_bind_group, &[]);

        for (&(cx, cy, cz), gpu_mesh) in chunk_meshes.iter() {
            let (min, max) = chunk_aabb(cx, cy, cz);
            if !frustum.contains_aabb(min, max) {
                stats.culled += 1;
                continue;
            }
            pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
            pass.set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..gpu_mesh.index_count, 0, 0..1);
            stats.draw_calls += 1;
        }
    }

    // --- Pass 1.2: Instanced plants (cross billboards, alpha cutout) ---
    // After chunks (so depth is set) and before water. Shares the camera +
    // texture bind groups; each chunk's instance buffer is drawn against the
    // shared unit cross. Writes depth so plants occlude correctly.
    if !plant_meshes.is_empty() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("plant_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(plant_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_bind_group(1, texture_bind_group, &[]);
        pass.set_vertex_buffer(0, plant_geo_vbuf.slice(..));
        pass.set_index_buffer(plant_geo_ibuf.slice(..), wgpu::IndexFormat::Uint32);

        for (&(cx, cy, cz), plants) in plant_meshes.iter() {
            let (min, max) = chunk_aabb(cx, cy, cz);
            if !frustum.contains_aabb(min, max) {
                stats.culled += 1;
                continue;
            }
            pass.set_vertex_buffer(1, plants.buffer.slice(..));
            pass.draw_indexed(0..plant_geo_index_count, 0, 0..plants.count);
            stats.draw_calls += 1;
        }
    }

    // --- Pass 1.25: Instanced micro-models (owner-inbox #18) ---
    // After plants (depth set), before decals/water. Shares the camera + texture
    // bind groups. Unlike plants (one shared cross), each micro-model TYPE has its
    // own baked shell geometry, so geometry is bound per type inside the per-chunk
    // loop. One instanced draw per (chunk, type) — same draw-call class as plants.
    // Opaque (depth write); frustum-culled per chunk.
    if !micro_meshes.is_empty() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("micro_model_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_bind_group(1, texture_bind_group, &[]);

        // Phase D LOD — NEAR chunks draw the full 3D shell (per-type baked
        // geometry on the micro pipeline). The frustum-cull increment lives here
        // (counted once); the far loop below re-checks the frustum silently.
        pass.set_pipeline(micro_pipeline);
        for (&(cx, cy, cz), batches) in micro_meshes.iter() {
            let (min, max) = chunk_aabb(cx, cy, cz);
            if !frustum.contains_aabb(min, max) {
                stats.culled += 1;
                continue;
            }
            if !crate::camera::micro_chunk_is_near(&gpu.last_view_proj, cx, cy, cz, micro_lod_dist) {
                continue; // handled by the far/billboard loop
            }
            for (block_id, insts) in batches {
                if let Some(geo) = micro_geo.get(block_id) {
                    pass.set_vertex_buffer(0, geo.vertex_buffer.slice(..));
                    pass.set_index_buffer(geo.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.set_vertex_buffer(1, insts.buffer.slice(..));
                    pass.draw_indexed(0..geo.index_count, 0, 0..insts.count);
                    stats.draw_calls += 1;
                }
            }
        }

        // Phase D LOD — FAR chunks fall back to the cheap cross-billboard: the
        // plant pipeline + shared cross geometry + the dedicated per-chunk
        // `micro_billboard_meshes` buffer (the block's ORIGINAL inset billboard,
        // so the far flower keeps its pre-#18 footprint — not a full-block cross).
        // One instanced draw per far chunk. Style still changes at the seam (3D
        // shell ↔ sprite); a pop-free cross-fade is deferred polish per the doc.
        pass.set_pipeline(plant_pipeline);
        pass.set_vertex_buffer(0, plant_geo_vbuf.slice(..));
        pass.set_index_buffer(plant_geo_ibuf.slice(..), wgpu::IndexFormat::Uint32);
        for (&(cx, cy, cz), billboards) in micro_billboard_meshes.iter() {
            let (min, max) = chunk_aabb(cx, cy, cz);
            if !frustum.contains_aabb(min, max) {
                continue; // already counted in the near loop
            }
            if crate::camera::micro_chunk_is_near(&gpu.last_view_proj, cx, cy, cz, micro_lod_dist) {
                continue; // drawn as a 3D shell above
            }
            pass.set_vertex_buffer(1, billboards.buffer.slice(..));
            pass.draw_indexed(0..plant_geo_index_count, 0, 0..billboards.count);
            stats.draw_calls += 1;
        }
    }

    // --- Pass 1.3: Wallpaper face-overlay decals (owner-inbox #1/2/3) ---
    // After plants, before water. Single-face quads sitting ~0.003 proud of the
    // wall with a negative depth bias (the crack-overlay technique) so they hug
    // the surface without z-fighting. Depth read-only (no write) — thin overlays
    // on already-drawn opaque walls. Same per-chunk frustum cull as plants.
    if !decal_meshes.is_empty() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("decal_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(decal_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_bind_group(1, texture_bind_group, &[]);
        for (&(cx, cy, cz), decal) in decal_meshes.iter() {
            let (min, max) = chunk_aabb(cx, cy, cz);
            if !frustum.contains_aabb(min, max) {
                stats.culled += 1;
                continue;
            }
            pass.set_vertex_buffer(0, decal.vertex_buffer.slice(..));
            pass.draw(0..decal.vertex_count, 0..1);
            stats.draw_calls += 1;
        }
    }

    // --- Pass 1.4: Exhibits — hung hi-res paintings ---
    // Opaque world quads, each with its OWN native-resolution texture (group 2),
    // drawn after walls so they occlude correctly and write depth. Group 1 binds
    // the block texture as an unused filler (the layout requires it; fs_painting
    // only reads groups 0 + 2). Per-painting frustum cull. Empty in any world
    // without exhibits, so this whole pass is skipped at zero cost.
    if !paintings.is_empty() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("painting_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(painting_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_bind_group(1, texture_bind_group, &[]); // filler — required by the layout
        for quad in paintings {
            if !frustum.contains_aabb(quad.aabb_min, quad.aabb_max) {
                stats.culled += 1;
                continue;
            }
            // Skip quads whose shared image hasn't finished uploading yet (async
            // WASM load): they pop in as their texture arrives.
            let Some(tex) = painting_textures.get(&quad.tex_id) else {
                continue;
            };
            pass.set_bind_group(2, &tex.bind_group, &[]);
            pass.set_vertex_buffer(0, quad.vertex_buffer.slice(..));
            pass.set_index_buffer(quad.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..quad.index_count, 0, 0..1);
            stats.draw_calls += 1;
        }
    }

    // --- Pass 1.4b: Exhibits — standing billboards (alpha cut-out) ---
    // Same hi-res textures as paintings (group 2), drawn with the alpha-discard
    // fragment so transparent PNG cut-outs read as objects. Corners are rewritten
    // each frame by `reorient_billboards` (Y-axis facing). Per-billboard frustum
    // cull. Empty (skipped) when no standing exhibit is present.
    if !billboards.is_empty() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("billboard_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(painting_alpha_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_bind_group(1, texture_bind_group, &[]); // filler — required by the layout
        for quad in billboards {
            if !frustum.contains_aabb(quad.aabb_min, quad.aabb_max) {
                stats.culled += 1;
                continue;
            }
            let Some(tex) = painting_textures.get(&quad.tex_id) else {
                continue;
            };
            pass.set_bind_group(2, &tex.bind_group, &[]);
            pass.set_vertex_buffer(0, quad.vertex_buffer.slice(..));
            pass.set_index_buffer(quad.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..quad.index_count, 0, 0..1);
            stats.draw_calls += 1;
        }
    }

    // --- Pass 1.5: Water (alpha blended) ---
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("water_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);

        // #131 — solid+transparent blocks (glass, …) draw first in this same
        // alpha-blended, depth-read-only pass (frustum-culled per chunk).
        pass.set_pipeline(transparent_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_bind_group(1, texture_bind_group, &[]);
        for (&(cx, cy, cz), gpu_mesh) in transparent_meshes.iter() {
            let (min, max) = chunk_aabb(cx, cy, cz);
            if !frustum.contains_aabb(min, max) {
                stats.culled += 1;
                continue;
            }
            pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
            pass.set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..gpu_mesh.index_count, 0, 0..1);
            stats.draw_calls += 1;
        }

        pass.set_pipeline(water_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_bind_group(1, texture_bind_group, &[]);

        for (&(cx, cy, cz), gpu_mesh) in water_meshes.iter() {
            let (min, max) = chunk_aabb(cx, cy, cz);
            if !frustum.contains_aabb(min, max) {
                stats.culled += 1;
                continue;
            }
            pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
            pass.set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..gpu_mesh.index_count, 0, 0..1);
            stats.draw_calls += 1;
        }
    }

    // --- Pass 1.6: Particles (2026-07-05) — alpha-blended camera-facing
    // billboards; depth read-only so terrain/water occlude them correctly.
    if particle_count > 0 {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("particle_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(particle_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_bind_group(1, texture_bind_group, &[]);
        pass.set_vertex_buffer(0, particle_geo_vbuf.slice(..));
        pass.set_vertex_buffer(1, particle_instance_buffer.slice(..));
        pass.set_index_buffer(particle_geo_ibuf.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..particle_geo_index_count, 0, 0..particle_count);
        stats.draw_calls += 1;
    }

    // --- Pass 1.7: Entities (textured multi-part models) ---
    if let Some(entity_buf) = gpu.entity_buffer.as_ref() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("entity_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(entity_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_bind_group(1, texture_bind_group, &[]);
        pass.set_vertex_buffer(0, entity_buf.slice(..));
        pass.draw(0..gpu.entity_vertex_count, 0..1);
    }

    // --- Pass 1.8: Player avatars (64x64 skin atlas) ---
    // Same depth target + load semantics as the entity pass; runs right after
    // it so avatars depth-sort against mobs/items. The avatar pipeline layout
    // declares group 1 (block texture) even though `fs_avatar` only samples
    // group 2 (the skin), so all three bind groups MUST be set — wgpu validates
    // every declared group regardless of shader use.
    if let Some(avatar_buf) = gpu.avatar_buffer.as_ref() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("avatar_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(avatar_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_bind_group(1, texture_bind_group, &[]);
        pass.set_bind_group(2, skin_bind_group, &[]);
        pass.set_vertex_buffer(0, avatar_buf.slice(..));
        pass.draw(0..gpu.avatar_vertex_count, 0..1);
    }

    // --- Pass 2: Block highlight wireframe ---
    if let Some(wire_buf) = gpu.wire_buffer.as_ref() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("wire_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store, // Must Store — Discard would corrupt depth for the next viewport
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(wire_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, wire_buf.slice(..));
        pass.draw(0..gpu.wire_vertex_count, 0..1);
    }

    // --- Pass 2.5: Spec 24 Phase 8 ghost wireframe ---
    if let Some(ghost_buf) = gpu.ghost_buffer.as_ref() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ghost_wire_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(wire_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, ghost_buf.slice(..));
        pass.draw(0..gpu.ghost_vertex_count, 0..1);
    }

    // --- Pass 2.5b: #8 spawn-proof overlay (red wireframe markers) ---
    if let Some(spawn_buf) = gpu.spawn_marker_buffer.as_ref() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("spawn_marker_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(wire_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, spawn_buf.slice(..));
        pass.draw(0..gpu.spawn_marker_vertex_count, 0..1);
    }

    // --- Pass 2.5b2: Trials chase-ghost (translucent wireframe runner) ---
    if let Some(ghost_buf) = gpu.trial_ghost_buffer.as_ref() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("trial_ghost_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(wire_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, ghost_buf.slice(..));
        pass.draw(0..gpu.trial_ghost_vertex_count, 0..1);
    }

    // --- Pass 2.5c: #9 blueprint build-guide ghosts (per-cell status colour) ---
    if let Some(guide_buf) = gpu.build_guide_buffer.as_ref() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("build_guide_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(wire_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, guide_buf.slice(..));
        pass.draw(0..gpu.build_guide_vertex_count, 0..1);
    }

    // --- Spec 48 Phase 2: visible IR beams (Beam Sensor) ---
    if let Some(beam_buf) = gpu.beam_buffer.as_ref() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("beam_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(wire_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, beam_buf.slice(..));
        pass.draw(0..gpu.beam_vertex_count, 0..1);
    }

    // --- Skin-painter precision aids: texel grid + hover footprint ---
    // Same pipeline/blend/depth as the other wire overlays, drawn last of them
    // so the yellow footprint box sits on top of the grid it shares edges with.
    if let Some(aid_buf) = gpu.paint_aid_buffer.as_ref() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("paint_aid_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(wire_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, aid_buf.slice(..));
        pass.draw(0..gpu.paint_aid_vertex_count, 0..1);
    }

    // --- Pass 2.6: Block-break crack overlay (Spec 05 §2.2) ---
    // Textured cube over the block the player is mining. Runs after the wire
    // passes and BEFORE the viewmodel (which clears depth) so it depth-tests
    // against world depth and is hidden behind nearer terrain. Alpha-blended;
    // the crack texture's own alpha controls how much it darkens the face.
    if let Some(crack_buf) = gpu.crack_buffer.as_ref() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("crack_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(crack_pipeline);
        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_bind_group(1, texture_bind_group, &[]);
        pass.set_vertex_buffer(0, crack_buf.slice(..));
        pass.draw(0..gpu.crack_vertex_count, 0..1);
    }

    // --- Pass 2.8: First-person viewmodel (Phase 7 / P1-T5) ---
    // MUST run AFTER the wire passes (Pass 2 / 2.5): this pass clears the
    // shared depth texture (LoadOp::Clear ignores the scissor and wipes the
    // ENTIRE attachment), so the block-highlight + ghost wireframes — which
    // Load world depth and depth-test LessEqual against it — would draw
    // through solid terrain if the viewmodel ran first. As the last 3D pass,
    // the depth clear only affects the viewmodel itself; nothing after it
    // (crosshair / UI) reads world depth, so world depth stays valid for the
    // wire passes above.
    //
    // Colour is kept (Load) but depth is CLEARED ONCE, so the held tool always
    // sits on top of the world and never z-fights with whatever the camera is
    // looking at. Geometry is already in view space, so everything binds the
    // viewmodel camera uniform (identity view + narrow-FOV projection) at
    // group 0, NOT the world camera.
    //
    // Two draws share this ONE depth-cleared pass so the arm and the held item
    // self-occlude correctly against the once-cleared depth (both pipelines
    // depth-test Less, depth-write on):
    //   1. the ARM — `avatar_pipeline` against the 64x64 skin texture (group 2),
    //      so an uploaded skin (Phase 2) shows on the player's own hand;
    //   2. the HELD item — `entity_pipeline` against the block texture array.
    // A second Clear would wipe the arm's depth; a second Load would be a
    // redundant extra pass — so both draws stay inside this single pass. The
    // arm buffer is the usual trigger (always present whenever a viewmodel
    // shows); the held buffer is absent when the hand is empty.
    if crate::viewmodel::SHOW_VIEWMODEL
        && (gpu.viewmodel_skin_buffer.is_some() || gpu.viewmodel_buffer.is_some())
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("viewmodel_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    // Clear depth so the viewmodel renders in front of the
                    // world; Store so the cleared depth doesn't leak corrupt
                    // values into the next viewport's Load passes.
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);

        // 1. ARM — avatar pipeline against the skin texture (group 2). The
        // avatar layout declares group 1 (block texture) too, so all three
        // bind groups must be set even though `fs_avatar` only samples group 2.
        if let Some(skin_buf) = gpu.viewmodel_skin_buffer.as_ref() {
            pass.set_pipeline(avatar_pipeline);
            pass.set_bind_group(0, &gpu.viewmodel_camera_bind_group, &[]);
            pass.set_bind_group(1, texture_bind_group, &[]);
            pass.set_bind_group(2, skin_bind_group, &[]);
            pass.set_vertex_buffer(0, skin_buf.slice(..));
            pass.draw(0..gpu.viewmodel_skin_vertex_count, 0..1);
        }

        // 2. HELD item — entity pipeline against the block texture array. Drawn
        // in the SAME pass against the once-cleared depth so it self-occludes
        // with the arm.
        if let Some(vm_buf) = gpu.viewmodel_buffer.as_ref() {
            pass.set_pipeline(entity_pipeline);
            pass.set_bind_group(0, &gpu.viewmodel_camera_bind_group, &[]);
            pass.set_bind_group(1, texture_bind_group, &[]);
            pass.set_vertex_buffer(0, vm_buf.slice(..));
            pass.draw(0..gpu.viewmodel_vertex_count, 0..1);
        }
    }

    // --- Pass 3: Crosshair overlay (per-viewport, centred) ---
    // Buffer is cached by viewport size in `Renderer.crosshair_cache` and passed
    // in — no per-frame allocation (Spec 39 A2). Skipped when the Director's
    // hide-HUD is on (clean cinematic frame) — the crosshair is its own pass, not
    // part of draw_hud, so the egui HUD gate doesn't cover it.
    if show_crosshair {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("crosshair_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });

        pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        pass.set_pipeline(crosshair_pipeline);
        pass.set_vertex_buffer(0, crosshair_buf.slice(..));
        pass.draw(0..12, 0..1);
    }

    stats
}

/// Build a viewmodel camera uniform buffer + bind group (Phase 7). The
/// viewmodel mesh is already in view space, so the uniform is filled each
/// frame with `view = identity` and a narrow-FOV perspective projection; this
/// just allocates the GPU resources against the shared camera bind-group
/// layout (same layout the world camera uses, so the entity pipeline binds it
/// at group 0 with no change).
fn make_viewmodel_camera(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let uniform = CameraUniform {
        view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
        camera_pos: [0.0, 0.0, 0.0, 0.0],
        sun_dir: [0.3, 1.0, 0.5, 1.0],
        fog: crate::camera::default_fog(),
        params: [1.0, 0.0, 0.0, 0.0],
        cam_right: [1.0, 0.0, 0.0, 0.0],
        cam_up: [0.0, 1.0, 0.0, 0.0],
    };
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("viewmodel_camera_uniform"),
        contents: bytemuck::cast_slice(&[uniform]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewmodel_camera_bind_group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    (buffer, bind_group)
}

/// Reserved skin-array layer for the #17 avatar preview. Exclusive: player
/// skins cap at `SKIN_LAYERS - 2` (`pure_helpers::skin_layer_of`), so staging
/// the previewed skin here never clobbers a player's skin and the in-world
/// avatars (layers 0..=SKIN_LAYERS-2) are untouched.
pub(crate) const SKIN_PREVIEW_LAYER: u32 = SKIN_LAYERS - 1;

/// Does this thumbnail need a fresh offscreen render? True when nothing is
/// cached for the entry yet, or the cached content key differs from the
/// entry's current `skin_key` (i.e. its pixels changed). Pure so it's unit-
/// testable without a GPU.
fn thumb_needs_render(cached: Option<u64>, current: u64) -> bool {
    cached != Some(current)
}

/// One cached 3D mini-avatar thumbnail for the "Your look" wardrobe grid: a small
/// (96×144), one-per-`SkinId` offscreen target. The colour view is registered
/// with egui once (`egui_id`) and re-rendered only when `cached_key` (the skin's
/// `skin_key`) changes.
struct SkinThumb {
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    vbuf: Option<wgpu::Buffer>,
    vcount: u32,
    egui_id: egui::TextureId,
    cached_key: u64,
    /// Arm model the cached image was rendered with — flipping Classic↔Slim
    /// changes the picture without changing the pixels, so the skin_key alone
    /// would serve a stale thumbnail.
    cached_arm: crate::skin_uv::ArmModel,
}

/// Default orbit framing for the small "Your look" preview (preserves the
/// pre-pitch look: a slight downward gaze at the chest from 2.6 units).
pub(crate) const PREVIEW_DEFAULT_PITCH: f32 = 0.06;
pub(crate) const PREVIEW_DEFAULT_DIST: f32 = 2.6;

/// Orbit-camera eye for the avatar preview/studio: orbit the chest target by
/// `yaw`/`pitch`, `dist` units out. Pitch clamped to ±~80°, dist to 1.2..5.0 so
/// the camera never enters the body or flies away.
fn skin_preview_eye(yaw: f32, pitch: f32, dist: f32) -> glam::Vec3 {
    const TARGET: glam::Vec3 = glam::Vec3::new(0.0, 0.95, 0.0);
    let p = pitch.clamp(-1.4, 1.4);
    let d = dist.clamp(1.2, 5.0);
    let dir = glam::Vec3::new(yaw.sin() * p.cos(), p.sin(), yaw.cos() * p.cos());
    TARGET + dir * d
}

/// View-projection for the avatar preview/studio: orbit camera looking at the
/// avatar's chest with a perspective projection at the given aspect ratio.
pub(crate) fn skin_preview_view_proj(yaw: f32, pitch: f32, dist: f32, aspect: f32) -> glam::Mat4 {
    let target = glam::Vec3::new(0.0, 0.95, 0.0);
    let view = glam::camera::rh::view::look_at_mat4(skin_preview_eye(yaw, pitch, dist), target, glam::Vec3::Y);
    let proj = glam::camera::rh::proj::directx::perspective(45f32.to_radians(), aspect.max(0.01), 0.05, 50.0);
    proj * view
}

fn create_depth_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth_texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Build the render-scale blit pipeline + sampler + bind-group layout (Spec 39
/// Phase 5). The pipeline samples an offscreen world target with a fullscreen
/// triangle (no vertex buffer) and upscales it to the surface.
fn build_blit_resources(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::Sampler, wgpu::BindGroupLayout) {
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("blit_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("blit_pipeline_layout"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("blit_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_blit"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_blit"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("blit_sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    (pipeline, sampler, bgl)
}

/// Create an offscreen world render target (color + depth) at `width`×`height`
/// plus the blit bind group that samples it (Spec 39 Phase 5).
fn create_offscreen_target(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    bgl: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    width: u32,
    height: u32,
) -> (wgpu::TextureView, wgpu::TextureView, wgpu::BindGroup) {
    let w = width.max(1);
    let h = height.max(1);
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen_world_color"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_view = create_depth_texture(device, w, h);
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("blit_bind_group"),
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&color_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    (color_view, depth_view, bind_group)
}

/// Write `bytes` into a reusable, grow-only vertex buffer (Spec 39 A2). Reuses
/// the existing GPU buffer when it is large enough (via `queue.write_buffer`),
/// avoiding the per-frame `create_buffer_init` allocator churn that was worst on
/// WASM/WebGPU. Reallocates only when the data outgrows the current capacity,
/// with ~25% headroom + `COPY_DST` so the next write can reuse it again. The
/// caller tracks the live vertex count separately, so any leftover capacity
/// beyond the written bytes is never drawn.
fn write_dynamic_vbuf(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    slot: &mut Option<wgpu::Buffer>,
    label: &'static str,
    bytes: &[u8],
) {
    let needed = bytes.len() as wgpu::BufferAddress;
    let fits = slot.as_ref().is_some_and(|b| b.size() >= needed);
    if !fits {
        let cap = (needed + needed / 4)
            .max(wgpu::COPY_BUFFER_ALIGNMENT)
            .next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
        *slot = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: cap,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
    queue.write_buffer(slot.as_ref().unwrap(), 0, bytes);
}

fn create_crosshair_buffer(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Buffer {
    let _aspect = width as f32 / height.max(1) as f32;
    // Size in pixels, converted to NDC (compensate for aspect ratio)
    let px = 1.0 / width as f32;
    let py = 1.0 / height as f32;
    let thickness = 2.0; // pixels
    let length = 12.0;   // pixels
    let w_x = thickness * px;
    let w_y = thickness * py;
    let l_x = length * px;
    let l_y = length * py;
    let c = [1.0, 1.0, 1.0, 0.8];

    let verts: Vec<CrosshairVertex> = vec![
        // Horizontal bar
        CrosshairVertex { position: [-l_x, -w_y], color: c },
        CrosshairVertex { position: [l_x, -w_y], color: c },
        CrosshairVertex { position: [l_x, w_y], color: c },
        CrosshairVertex { position: [-l_x, -w_y], color: c },
        CrosshairVertex { position: [l_x, w_y], color: c },
        CrosshairVertex { position: [-l_x, w_y], color: c },
        // Vertical bar
        CrosshairVertex { position: [-w_x, -l_y], color: c },
        CrosshairVertex { position: [w_x, -l_y], color: c },
        CrosshairVertex { position: [w_x, l_y], color: c },
        CrosshairVertex { position: [-w_x, -l_y], color: c },
        CrosshairVertex { position: [w_x, l_y], color: c },
        CrosshairVertex { position: [-w_x, l_y], color: c },
    ];

    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("crosshair_buffer"),
        contents: bytemuck::cast_slice(&verts),
        usage: wgpu::BufferUsages::VERTEX,
    })
}

/// Spec 05 §2.2 — depth-biased overlay pipeline. Reuses the chunk shader's
/// `vs_main` (so it samples the block texture array) with a caller-chosen
/// fragment (`fs_crack` for block-break cracks, `fs_decal` for wallpaper face
/// overlays — owner-inbox #1/2/3), the chunk pipeline layout (camera + texture
/// bind groups), and the `Vertex` format. Alpha-blended, no culling, depth
/// read-only with the same negative bias as the wireframe so the overlay hugs
/// the block surface and stays correctly occluded by nearer terrain.
fn make_overlay_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    fs_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Vertex::layout()],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fs_entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState {
                constant: -2,
                slope_scale: -1.0,
                clamp: 0.0,
            },
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Spec 05 §2.2 — build the 6-face textured cube for the crack overlay at a
/// block position, inflated slightly so it sits just outside the block surface.
/// All faces sample crack-stage layer `tex_layer`; full-face UVs; `light = 1.0`
/// (the crack fragment shader is unlit anyway).
fn build_crack_cube(pos: [i32; 3], tex_layer: u32) -> Vec<Vertex> {
    let e = 0.003_f32;
    let x0 = pos[0] as f32 - e;
    let y0 = pos[1] as f32 - e;
    let z0 = pos[2] as f32 - e;
    let x1 = pos[0] as f32 + 1.0 + e;
    let y1 = pos[1] as f32 + 1.0 + e;
    let z1 = pos[2] as f32 + 1.0 + e;

    let mut verts: Vec<Vertex> = Vec::with_capacity(36);
    // Each face: 4 corners (CCW) + UVs + normal, emitted as two triangles.
    let mut face = |corners: [[f32; 3]; 4], normal: [f32; 3]| {
        let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        let mk = |i: usize| Vertex {
            position: corners[i],
            normal,
            tex_layer,
            uv: uvs[i],
            light: 1.0,
            sky_light: 0.0,
        };
        verts.extend([mk(0), mk(1), mk(2), mk(0), mk(2), mk(3)]);
    };

    // +Y (top)
    face([[x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0]], [0.0, 1.0, 0.0]);
    // -Y (bottom)
    face([[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]], [0.0, -1.0, 0.0]);
    // +Z (south)
    face([[x0, y1, z1], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1]], [0.0, 0.0, 1.0]);
    // -Z (north)
    face([[x1, y1, z0], [x1, y0, z0], [x0, y0, z0], [x0, y1, z0]], [0.0, 0.0, -1.0]);
    // +X (east)
    face([[x1, y1, z1], [x1, y0, z1], [x1, y0, z0], [x1, y1, z0]], [1.0, 0.0, 0.0]);
    // -X (west)
    face([[x0, y1, z0], [x0, y0, z0], [x0, y0, z1], [x0, y1, z1]], [-1.0, 0.0, 0.0]);

    verts
}

fn build_wireframe_cube(pos: [i32; 3]) -> Vec<WireVertex> {
    build_wireframe_cube_colored(pos, [0.1, 0.1, 0.1, 0.6])
}

/// Spec 24 Phase 8 — same edge geometry as `build_wireframe_cube` but
/// caller-tinted. Pulled out so ghost cubes can colour-code per
/// validity without duplicating the edge maths.
fn build_wireframe_cube_colored(pos: [i32; 3], c: [f32; 4]) -> Vec<WireVertex> {
    build_wireframe_cube_colored_t(pos, c, 0.01)
}

/// Same as [`build_wireframe_cube_colored`] but with an explicit edge
/// half-thickness `t` (world units). The blow-up cage draws 64 adjacent cells
/// whose shared edges overlap, so it passes a thinner `t` than the single
/// block-highlight cube to keep the grid from reading as a heavy lattice.
fn build_wireframe_cube_colored_t(pos: [i32; 3], c: [f32; 4], t: f32) -> Vec<WireVertex> {
    let x = pos[0] as f32;
    let y = pos[1] as f32;
    let z = pos[2] as f32;
    let e = 0.002;

    let (x0, y0, z0) = (x - e, y - e, z - e);
    let (x1, y1, z1) = (x + 1.0 + e, y + 1.0 + e, z + 1.0 + e);

    let mut verts = Vec::with_capacity(12 * 6);

    let mut edge = |ax: f32, ay: f32, az: f32, bx: f32, by: f32, bz: f32| {
        let dx = (bx - ax).abs();
        let dy = (by - ay).abs();
        let dz = (bz - az).abs();

        let (ox, oy, oz) = if dx > dy && dx > dz {
            (0.0, t, t)
        } else if dy > dz {
            (t, 0.0, t)
        } else {
            (t, t, 0.0)
        };

        verts.push(WireVertex { position: [ax - ox, ay - oy, az - oz], color: c });
        verts.push(WireVertex { position: [bx - ox, by - oy, bz - oz], color: c });
        verts.push(WireVertex { position: [bx + ox, by + oy, bz + oz], color: c });
        verts.push(WireVertex { position: [ax - ox, ay - oy, az - oz], color: c });
        verts.push(WireVertex { position: [bx + ox, by + oy, bz + oz], color: c });
        verts.push(WireVertex { position: [ax + ox, ay + oy, az + oz], color: c });
    };

    edge(x0, y0, z0, x1, y0, z0);
    edge(x1, y0, z0, x1, y0, z1);
    edge(x1, y0, z1, x0, y0, z1);
    edge(x0, y0, z1, x0, y0, z0);

    edge(x0, y1, z0, x1, y1, z0);
    edge(x1, y1, z0, x1, y1, z1);
    edge(x1, y1, z1, x0, y1, z1);
    edge(x0, y1, z1, x0, y1, z0);

    edge(x0, y0, z0, x0, y1, z0);
    edge(x1, y0, z0, x1, y1, z0);
    edge(x1, y0, z1, x1, y1, z1);
    edge(x0, y0, z1, x0, y1, z1);

    verts
}

/// Expand one arbitrary world-space line into a thin CROSS of two perpendicular
/// quads (12 vertices) on the TriangleList wire pipeline.
///
/// `build_wireframe_box_t`'s single-quad trick relies on its edges being axis
/// aligned; the skin-painter aids ride a mannequin that can be rotated to any
/// yaw and their lines run in arbitrary directions, so the offset basis is
/// derived from the line itself. Two quads rather than one because a single
/// quad vanishes when viewed edge-on — exactly the grazing angle a painter uses
/// on the side of an arm.
fn push_thick_line(verts: &mut Vec<WireVertex>, a: [f32; 3], b: [f32; 3], c: [f32; 4], t: f32) {
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if len < 1e-6 {
        return; // degenerate — nothing to draw
    }
    let u = [d[0] / len, d[1] / len, d[2] / len];
    // Any axis that isn't (nearly) parallel to the line keeps the cross stable.
    let helper = if u[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let cross = |p: [f32; 3], q: [f32; 3]| {
        [
            p[1] * q[2] - p[2] * q[1],
            p[2] * q[0] - p[0] * q[2],
            p[0] * q[1] - p[1] * q[0],
        ]
    };
    let mut n1 = cross(u, helper);
    let n1_len = (n1[0] * n1[0] + n1[1] * n1[1] + n1[2] * n1[2]).sqrt();
    if n1_len < 1e-6 {
        return;
    }
    n1 = [n1[0] / n1_len, n1[1] / n1_len, n1[2] / n1_len];
    let n2 = cross(u, n1); // already unit: u ⟂ n1, both unit
    for n in [n1, n2] {
        let o = [n[0] * t, n[1] * t, n[2] * t];
        let am = [a[0] - o[0], a[1] - o[1], a[2] - o[2]];
        let ap = [a[0] + o[0], a[1] + o[1], a[2] + o[2]];
        let bm = [b[0] - o[0], b[1] - o[1], b[2] - o[2]];
        let bp = [b[0] + o[0], b[1] + o[1], b[2] + o[2]];
        verts.push(WireVertex { position: am, color: c });
        verts.push(WireVertex { position: bm, color: c });
        verts.push(WireVertex { position: bp, color: c });
        verts.push(WireVertex { position: am, color: c });
        verts.push(WireVertex { position: bp, color: c });
        verts.push(WireVertex { position: ap, color: c });
    }
}

/// A coloured wireframe box at an arbitrary FLOAT position + size (not block-
/// aligned) — for the Trials chase-ghost, which moves continuously. Same edge
/// geometry as `build_wireframe_cube_colored_t`, just with explicit min + size.
fn build_wireframe_box_t(min: [f32; 3], size: [f32; 3], c: [f32; 4], t: f32) -> Vec<WireVertex> {
    let (x0, y0, z0) = (min[0], min[1], min[2]);
    let (x1, y1, z1) = (min[0] + size[0], min[1] + size[1], min[2] + size[2]);
    let mut verts = Vec::with_capacity(12 * 6);
    let mut edge = |ax: f32, ay: f32, az: f32, bx: f32, by: f32, bz: f32| {
        let dx = (bx - ax).abs();
        let dy = (by - ay).abs();
        let dz = (bz - az).abs();
        let (ox, oy, oz) = if dx > dy && dx > dz {
            (0.0, t, t)
        } else if dy > dz {
            (t, 0.0, t)
        } else {
            (t, t, 0.0)
        };
        verts.push(WireVertex { position: [ax - ox, ay - oy, az - oz], color: c });
        verts.push(WireVertex { position: [bx - ox, by - oy, bz - oz], color: c });
        verts.push(WireVertex { position: [bx + ox, by + oy, bz + oz], color: c });
        verts.push(WireVertex { position: [ax - ox, ay - oy, az - oz], color: c });
        verts.push(WireVertex { position: [bx + ox, by + oy, bz + oz], color: c });
        verts.push(WireVertex { position: [ax + ox, ay + oy, az + oz], color: c });
    };
    edge(x0, y0, z0, x1, y0, z0);
    edge(x1, y0, z0, x1, y0, z1);
    edge(x1, y0, z1, x0, y0, z1);
    edge(x0, y0, z1, x0, y0, z0);
    edge(x0, y1, z0, x1, y1, z0);
    edge(x1, y1, z0, x1, y1, z1);
    edge(x1, y1, z1, x0, y1, z1);
    edge(x0, y1, z1, x0, y1, z0);
    edge(x0, y0, z0, x0, y1, z0);
    edge(x1, y0, z0, x1, y1, z0);
    edge(x1, y0, z1, x1, y1, z1);
    edge(x0, y0, z1, x0, y1, z1);
    verts
}

/// GPU resources for avatar rendering, shared by both `Renderer::new` and
/// `Renderer::new_headless` so the 64×64 skin pipeline is built identically on
/// real and headless devices.
struct AvatarResources {
    skin_texture: wgpu::Texture,
    skin_bind_group_layout: wgpu::BindGroupLayout,
    skin_bind_group: wgpu::BindGroup,
    avatar_pipeline: wgpu::RenderPipeline,
}

/// Number of layers in the avatar skin texture array — one per simultaneously
/// visible skin (local split-screen players + remote players). Layer 0 is the
/// local player's own skin (Phase 2 upload target); each visible player's avatar
/// verts carry their layer index in `Vertex.tex_layer` (`skin_layer_of(idx)`),
/// which `fs_avatar` samples. Players beyond the cap share the last layer.
pub(crate) const SKIN_LAYERS: u32 = 8;

/// Build the 64×64 skin texture, its bind group, and the avatar pipeline.
///
/// The skin texture uses the SAME format as the block array (`Rgba8UnormSrgb`)
/// so colours stay consistent, and reuses the caller's nearest sampler so
/// pixel-art skins don't blur. The pipeline mirrors `entity_pipeline`
/// (REPLACE blend, no cull, depth32 Less, depth write on) but binds three
/// groups — `[camera, texture, skin]` — with the skin at `@group(2)`. Group 1
/// (the block array) is kept in the layout even though `fs_avatar` never
/// samples it, so the next task can bind group1=texture / group2=skin without
/// rebinding gymnastics. `color_format` is the render target's format
/// (`surface_format` for the windowed device, the offscreen `format` headless).
/// Exhibits / paintings — build the painting pipeline, its group-2 bind-group layout, and
/// a linear sampler. Mirrors `create_avatar_resources`: a 3-group layout
/// (camera / block-texture / painting) reusing `vs_main`, with a dedicated
/// `fs_painting` fragment. Opaque, depth-writing (paintings occlude properly),
/// no back-face cull (visible from either side). Bindings 2/3 of group 2 keep it
/// clear of the avatar skin slots (0/1).
fn create_painting_resources(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    color_format: wgpu::TextureFormat,
    camera_bind_group_layout: &wgpu::BindGroupLayout,
    texture_bind_group_layout: &wgpu::BindGroupLayout,
) -> (
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::BindGroupLayout,
    wgpu::Sampler,
) {
    let painting_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("painting_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

    let painting_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("painting_sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let painting_pipeline_layout =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("painting_pipeline_layout"),
            // group 0 = camera, group 1 = block texture (unused by fs_painting but
            // present so draw-time binding mirrors the chunk/entity pipelines),
            // group 2 = the painting's own texture.
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(texture_bind_group_layout),
                Some(&painting_bind_group_layout),
            ],
            immediate_size: 0,
        });

    let painting_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("painting_render_pipeline"),
        layout: Some(&painting_pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Vertex::layout()],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_painting"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: color_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None, // a painting is visible from either side
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    // Exhibits — alpha-cutout variant: identical layout/sampler/state, but the
    // `fs_painting_alpha` fragment discards transparent texels so standing PNG
    // billboards read as cut-out objects rather than rectangles.
    let painting_alpha_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("painting_alpha_render_pipeline"),
        layout: Some(&painting_pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Vertex::layout()],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_painting_alpha"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: color_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    (
        painting_pipeline,
        painting_alpha_pipeline,
        painting_bind_group_layout,
        painting_sampler,
    )
}

fn create_avatar_resources(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    color_format: wgpu::TextureFormat,
    nearest_sampler: &wgpu::Sampler,
    camera_bind_group_layout: &wgpu::BindGroupLayout,
    texture_bind_group_layout: &wgpu::BindGroupLayout,
) -> AvatarResources {
    const SKIN_SIZE: u32 = 64;

    let skin_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("avatar_skin_texture"),
        size: wgpu::Extent3d {
            width: SKIN_SIZE,
            height: SKIN_SIZE,
            // One layer per local player (split-screen) / per remote skin.
            // Layer 0 is the local player's own skin (Phase 2 back-compat).
            depth_or_array_layers: SKIN_LAYERS,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // Match the block array so avatar colours render consistently.
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    // Seed EVERY layer with the default skin so any layer a player maps to
    // (before a real skin is written into it) shows the default rather than
    // garbage. Layer 0 is the local player's; split-screen seeds tints into 1+.
    let skin_pixels = crate::texture_gen::default_skin_rgba();
    for layer in 0..SKIN_LAYERS {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &skin_texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                aspect: wgpu::TextureAspect::All,
            },
            &skin_pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SKIN_SIZE * 4),
                rows_per_image: Some(SKIN_SIZE),
            },
            wgpu::Extent3d {
                width: SKIN_SIZE,
                height: SKIN_SIZE,
                depth_or_array_layers: 1,
            },
        );
    }

    let skin_view = skin_texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });

    let skin_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("skin_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

    let skin_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("skin_bind_group"),
        layout: &skin_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&skin_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(nearest_sampler),
            },
        ],
    });

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("avatar_shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
    });

    let avatar_pipeline_layout =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("avatar_pipeline_layout"),
            // group 0 = camera, group 1 = block texture (unused by fs_avatar but
            // kept so draw-time binding mirrors the chunk/entity pipelines),
            // group 2 = skin.
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(texture_bind_group_layout),
                Some(&skin_bind_group_layout),
            ],
            immediate_size: 0,
        });

    let avatar_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("avatar_render_pipeline"),
        layout: Some(&avatar_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            // Reuse the chunk/entity vertex stage verbatim — positions are
            // world-space, no model matrix.
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Vertex::layout()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_avatar"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: color_format,
                // Alpha-cutout in fs_avatar handles transparency; no blending.
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            // Match entity_pipeline — model faces visible from both sides.
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    AvatarResources {
        skin_texture,
        skin_bind_group_layout,
        skin_bind_group,
        avatar_pipeline,
    }
}

#[cfg(test)]
mod skin_preview_tests {
    use super::{skin_preview_eye, skin_preview_view_proj};

    #[test]
    fn skin_preview_camera_orbits_at_constant_radius() {
        // #17 — yaw 0 puts the camera in front (+z); yaw 90° swings it to the
        // side (+x); the orbit radius around the avatar's chest stays constant.
        let e0 = skin_preview_eye(0.0, 0.0, 2.6);
        assert!(e0.z > 2.0 && e0.x.abs() < 0.001, "yaw 0 faces +z: {e0:?}");
        let e90 = skin_preview_eye(std::f32::consts::FRAC_PI_2, 0.0, 2.6);
        assert!(e90.x > 2.0 && e90.z.abs() < 0.001, "yaw 90° swings to +x: {e90:?}");
        let target = glam::Vec3::new(0.0, 0.95, 0.0);
        let d0 = (e0 - target).length();
        let d90 = (e90 - target).length();
        assert!((d0 - d90).abs() < 0.001, "orbit keeps a constant radius");
        // view-proj is finite for a normal aspect (no NaN/inf reaching the GPU).
        let vp = skin_preview_view_proj(0.3, 0.0, 2.6, 256.0 / 384.0);
        assert!(vp.to_cols_array().iter().all(|f| f.is_finite()));
    }
}

#[cfg(test)]
mod skin_preview_cam_tests {
    use super::skin_preview_eye;

    const TARGET: glam::Vec3 = glam::Vec3::new(0.0, 0.95, 0.0);

    #[test]
    fn front_at_zero_yaw_pitch() {
        // yaw=0, pitch=0, dist=2.6 → directly in front (+Z) at target height.
        let e = skin_preview_eye(0.0, 0.0, 2.6);
        assert!((e - (TARGET + glam::Vec3::new(0.0, 0.0, 2.6))).length() < 1e-4, "got {e:?}");
    }

    #[test]
    fn pitch_up_raises_eye_and_clamps() {
        let level = skin_preview_eye(0.0, 0.0, 2.6).y;
        assert!(skin_preview_eye(0.0, 1.0, 2.6).y > level, "pitch up raises the eye");
        // Beyond the clamp the eye stays within `dist` of the target (no blow-up).
        let far_pitch = skin_preview_eye(0.0, 9.0, 2.6);
        assert!((far_pitch - TARGET).length() <= 2.6 + 1e-3, "dist preserved under pitch clamp");
    }

    #[test]
    fn dist_zoom_is_clamped() {
        // Too-near request is clamped to the min orbit radius (camera never enters the body).
        let near = skin_preview_eye(0.0, 0.0, 0.1);
        assert!((near - TARGET).length() >= 1.2 - 1e-3, "min zoom radius enforced, got {near:?}");
    }
}

#[cfg(test)]
mod surface_format_tests {
    use super::choose_surface_formats;
    use wgpu::TextureFormat as F;

    #[test]
    fn native_srgb_format_used_directly_no_alias() {
        // Native backends (Vulkan/Metal/DX12) expose an sRGB surface format.
        // We pick it for both config and render, with no view alias — i.e. the
        // pre-existing native behaviour is unchanged.
        let (cfg, render, vf) = choose_surface_formats(&[F::Bgra8UnormSrgb, F::Bgra8Unorm]);
        assert_eq!(cfg, F::Bgra8UnormSrgb);
        assert_eq!(render, F::Bgra8UnormSrgb);
        assert!(vf.is_empty(), "no sRGB view alias needed on native");
    }

    #[test]
    fn webgpu_nonsrgb_canvas_gets_srgb_render_alias() {
        // WebGPU only exposes non-sRGB canvas formats. The canvas must be
        // CONFIGURED with the non-sRGB format, but we RENDER through an sRGB view
        // alias (declared in view_formats) so linear output is gamma-encoded —
        // otherwise the scene is too dark (the WASM-only bug this fixes).
        let (cfg, render, vf) = choose_surface_formats(&[F::Bgra8Unorm, F::Rgba8Unorm]);
        assert_eq!(cfg, F::Bgra8Unorm, "canvas configured with its non-sRGB format");
        assert_eq!(render, F::Bgra8UnormSrgb, "but rendered through the sRGB alias");
        assert_eq!(vf, vec![F::Bgra8UnormSrgb], "alias must be in view_formats");
    }

    #[test]
    fn webgpu_rgba_canvas_also_aliased() {
        let (cfg, render, vf) = choose_surface_formats(&[F::Rgba8Unorm]);
        assert_eq!(cfg, F::Rgba8Unorm);
        assert_eq!(render, F::Rgba8UnormSrgb);
        assert_eq!(vf, vec![F::Rgba8UnormSrgb]);
    }

    #[test]
    fn srgb_render_format_equals_config_implies_empty_view_formats() {
        // Invariant relied on by `surface_view_formats()`: an empty alias list
        // exactly when render == config.
        for available in [
            vec![F::Bgra8UnormSrgb],
            vec![F::Bgra8Unorm],
            vec![F::Rgba8Unorm, F::Bgra8Unorm],
        ] {
            let (cfg, render, vf) = choose_surface_formats(&available);
            assert_eq!(vf.is_empty(), render == cfg);
        }
    }
}

#[cfg(test)]
mod shader_tests {
    /// Statically parse + validate `shader.wgsl` with naga (the same shader
    /// compiler wgpu uses at pipeline creation). Catches WGSL syntax/type
    /// errors in the shader **source** — including the instanced-plant
    /// `vs_plant`/`fs_plant` and the avatar `fs_avatar` (group-2 skin) entry
    /// points — without needing a GPU.
    ///
    /// SCOPE: this checks the WGSL source only. It does NOT exercise pipeline
    /// or bind-group-layout creation, so a mismatch between a Rust
    /// `BindGroupLayout` (e.g. `skin_bind_group_layout`) and the shader's
    /// `@group/@binding` decls — which only surfaces when the device builds the
    /// pipeline — would pass here. That binding-layout path is currently only
    /// exercised end-to-end by the browser smoke test in `check.sh`. (`new_headless`
    /// now lifts `max_texture_array_layers` to the adapter's max — Goal 3 / Task 3,
    /// matching the windowed path — so it CAN allocate the 506-layer block atlas on a
    /// fallback adapter; a headless device-level test is feasible but not pursued here.)
    #[test]
    fn wgsl_shader_validates() {
        let src = include_str!("shader.wgsl");
        let module = naga::front::wgsl::parse_str(src)
            .unwrap_or_else(|e| panic!("shader.wgsl parse error:\n{}", e.emit_to_string(src)));
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .unwrap_or_else(|e| panic!("shader.wgsl validation error: {e:?}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_backdrop_encodes_to_deep_frontier() {
        // sRGB view: the hardware re-encodes the linear clear value, so it must
        // land back on the brand bytes (13, 27, 30), not the old slate #424c63.
        let c = menu_backdrop_color(wgpu::TextureFormat::Bgra8UnormSrgb);
        let enc = |l: f64| -> u8 {
            let s = if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
            (s * 255.0).round() as u8
        };
        assert_eq!((enc(c.r), enc(c.g), enc(c.b)), (13, 27, 30));
        // Non-sRGB view: bytes are written as-is.
        let c = menu_backdrop_color(wgpu::TextureFormat::Bgra8Unorm);
        assert_eq!(((c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8), (13, 27, 30));
    }

    #[test]
    fn thumb_cache_invalidates_on_key_change() {
        // Never rendered yet → must render.
        assert!(thumb_needs_render(None, 7));
        // Same key cached → cache hit, skip render.
        assert!(!thumb_needs_render(Some(7), 7));
        // Bytes changed (new skin_key) → must re-render.
        assert!(thumb_needs_render(Some(7), 9));
        // Default skin (key 0) is a normal value, not a sentinel here.
        assert!(thumb_needs_render(None, 0));
        assert!(!thumb_needs_render(Some(0), 0));
    }

    // --- P6 audit: exhibit/gallery image larger than max_texture_dimension_2d
    // crashes the renderer. `clamp_image_dimensions` is the pure sizing logic
    // that `upload_painting_image` applies before ever calling
    // `device.create_texture`. ---

    #[test]
    fn clamp_image_dimensions_leaves_small_images_untouched() {
        assert_eq!(clamp_image_dimensions(800, 600, 8192, MAX_PAINTING_IMAGE_PIXELS), (800, 600));
    }

    #[test]
    fn clamp_image_dimensions_shrinks_to_the_per_axis_device_limit() {
        // A square image bigger than the device's max_texture_dimension_2d.
        let (w, h) = clamp_image_dimensions(10000, 10000, 4096, MAX_PAINTING_IMAGE_PIXELS);
        assert!(w <= 4096 && h <= 4096, "got {w}x{h}");
        assert_eq!(w, h, "aspect preserved on a square input");
    }

    #[test]
    fn clamp_image_dimensions_catches_the_audit_scenario_10000x10() {
        // The audit's literal repro: "a 10000×10 exhibit PNG" — a few hundred
        // bytes, well under the byte-size gate, but 10000 exceeds a typical
        // 8192 max_texture_dimension_2d on its long axis.
        let (w, h) = clamp_image_dimensions(10000, 10, 8192, MAX_PAINTING_IMAGE_PIXELS);
        assert!(w <= 8192, "long axis must be clamped, got {w}x{h}");
        assert!(h >= 1, "short axis never collapses to 0, got {w}x{h}");
        // Aspect ratio (10000:10 = 1000:1) preserved within rounding.
        let orig_aspect = 10000.0 / 10.0;
        let new_aspect = w as f64 / h as f64;
        assert!((orig_aspect - new_aspect).abs() / orig_aspect < 0.05, "aspect drifted: {w}x{h}");
    }

    #[test]
    fn clamp_image_dimensions_also_enforces_the_total_pixel_budget() {
        // Within the per-axis limit on both dimensions, but the product blows
        // the total-pixel budget — the audit's "cap the total pixels" ask,
        // independent of the per-axis clamp.
        let (w, h) = clamp_image_dimensions(8000, 8000, 8192, 4096 * 4096);
        let pixels = w as u64 * h as u64;
        assert!(pixels <= 4096 * 4096, "got {w}x{h} = {pixels} pixels");
    }

    #[test]
    fn clamp_image_dimensions_never_collapses_a_dimension_to_zero() {
        // A pathologically thin image must still clamp to at least 1px on
        // the short axis (a 0-width/height texture is itself invalid).
        let (w, h) = clamp_image_dimensions(1_000_000, 1, 8192, MAX_PAINTING_IMAGE_PIXELS);
        assert!(w >= 1 && h >= 1, "got {w}x{h}");
    }

    #[test]
    fn clamp_image_dimensions_passes_through_zero_size_unchanged() {
        // Zero width/height is handled by upload_painting_image's own
        // malformed-image branch (placeholder), not by this clamp — it must
        // not divide by zero or panic.
        assert_eq!(clamp_image_dimensions(0, 100, 8192, MAX_PAINTING_IMAGE_PIXELS), (0, 100));
        assert_eq!(clamp_image_dimensions(100, 0, 8192, MAX_PAINTING_IMAGE_PIXELS), (100, 0));
    }

    #[test]
    fn downscale_rgba_produces_the_requested_pixel_count() {
        let side = 64u32;
        let pixels: Vec<u8> = vec![200u8; (side * side * 4) as usize];
        let out = downscale_rgba(&pixels, side, side, 16, 16);
        assert_eq!(out.len(), 16 * 16 * 4);
    }

    #[test]
    fn placeholder_image_rgba_is_a_valid_square_buffer() {
        let px = placeholder_image_rgba();
        assert_eq!(px.len(), (PLACEHOLDER_IMAGE_SIDE * PLACEHOLDER_IMAGE_SIDE * 4) as usize);
        assert!(px.chunks_exact(4).all(|p| p[3] == 255), "placeholder is fully opaque");
    }
}

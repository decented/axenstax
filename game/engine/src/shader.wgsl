// Axe'n'Stax — Chunk shader with texture array sampling.
//
// Bind group 0: camera uniform (view_proj, camera_pos, sun_dir).
// Bind group 1: block texture array + sampler.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    sun_dir: vec4<f32>,       // xyz = sun direction, w = sky brightness 0-1
    // Spec 39 — distance fog derived from the live render distance (fixes the
    // pop-in bug). xy = terrain fog (start, end); zw = water fog (start, end).
    // Fog "off" pushes all four very far so nothing fades.
    fog: vec4<f32>,
    // x = user brightness control (graphics setting). yzw reserved.
    params: vec4<f32>,
    // Particle billboarding (2026-07-05): the camera's world-space right/up
    // (xyz; w unused). Per-player, so split-screen billboards face each eye.
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

// User brightness/gamma control (graphics-menu slider). Applies `pow(c, 1/x)`
// to a lit surface colour: x = 1.0 is neutral, x > 1 brightens (lifts shadows
// like a gamma slider), x < 1 darkens. x ≤ 0 (e.g. a zeroed uniform on a path
// that never set it) is treated as neutral, so brightness can never produce a
// black screen. Applied at every lit world-surface output; unlit overlays
// (crack/decal) are left alone.
fn apply_user_brightness(c: vec3<f32>) -> vec3<f32> {
    let x = camera.params.x;
    let inv = select(1.0, 1.0 / x, x > 0.001);
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(inv));
}

@group(1) @binding(0)
var block_textures: texture_2d_array<f32>;

@group(1) @binding(1)
var block_sampler: sampler;

// Bind group 2: per-avatar 64×64 skin texture + sampler. Sampled by
// `fs_avatar` (the avatar pipeline) instead of the shared block array, so the
// player model can carry a dedicated skin with sub-rect UVs. Nearest-filtered
// like the block sampler — pixel-art skins must not blur.
@group(2) @binding(0)
var skin_texture: texture_2d_array<f32>;

@group(2) @binding(1)
var skin_sampler: sampler;

// The Gallery — one hi-res painting texture per hung quad (its OWN 2D texture at
// the artwork's native resolution, NOT the 16×16 block array). Bindings 2/3 of
// group 2 are otherwise unused, so paintings sit alongside the avatar skin slots
// without a clash. Linear-filtered (smooth photographic art, unlike pixel skins).
@group(2) @binding(2)
var painting_tex: texture_2d<f32>;

@group(2) @binding(3)
var painting_samp: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_layer: u32,
    @location(3) uv: vec2<f32>,
    @location(4) light: f32,
    // P1 (2026-07-04): separate sky-light channel — scaled by time-of-day in
    // fs_main so nights darken sky-lit terrain while torches keep working.
    // Non-terrain paths pass 0 (their `light` already holds the final value).
    @location(5) sky_light: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) @interpolate(flat) tex_layer: u32,
    @location(3) uv: vec2<f32>,
    @location(4) light: f32,
    @location(5) sky_light: f32,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.normal = in.normal;
    out.world_pos = in.position;
    out.tex_layer = in.tex_layer;
    out.uv = in.uv;
    out.light = in.light;
    out.sky_light = in.sky_light;
    return out;
}

// --- Instanced plant billboards (flowers / grass / crops) ---
// Vertex buffer 0: shared unit cross (local position in [0,1]^3 + uv).
// Vertex buffer 1 (per instance): world-min corner, AABB size, texture
// layer, block light. The unit cross is scaled by `size` and offset by
// `pos` so each instance keeps its species' footprint.
struct PlantVertexInput {
    @location(0) local: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) inst_pos: vec3<f32>,
    @location(3) inst_size: vec3<f32>,
    @location(4) tex_layer: u32,
    @location(5) light: f32,
    @location(6) sky: f32,
};

@vertex
fn vs_plant(in: PlantVertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world = in.inst_pos + in.local * in.inst_size;
    out.clip_position = camera.view_proj * vec4<f32>(world, 1.0);
    out.normal = vec3<f32>(0.0, 1.0, 0.0); // up-facing → even, never self-shadowed
    out.world_pos = world;
    out.tex_layer = in.tex_layer;
    out.uv = in.uv;
    out.light = in.light;
    // Campaign N — sky rides its own channel so plants dim at night.
    out.sky_light = in.sky;
    return out;
}

// --- Instanced micro-models (owner-inbox #18) ---
// Vertex buffer 0: a baked sub-voxel shell's full `Vertex` (position in the host
// block's [0,1]^3 model space, real per-face normal, baked texture layer, uv,
// baked light). Vertex buffer 1 (per instance): host block min corner + unit
// size + (far-LOD) tex layer + placement light. Unlike vs_plant this keeps the
// shell's REAL per-vertex normal (so the 3D form gets directional shading) and
// takes the texture per-vertex from the shell (paint-with-blocks); the instance
// supplies only placement + light. Renders through fs_main (opaque, no cutout).
struct MicroVertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_layer: u32,
    @location(3) uv: vec2<f32>,
    @location(4) light: f32,
    @location(5) sky_light: f32,
    @location(6) inst_pos: vec3<f32>,
    @location(7) inst_size: vec3<f32>,
    @location(8) inst_tex_layer: u32,
    @location(9) inst_light: f32,
    @location(10) inst_sky: f32,
};

@vertex
fn vs_micro(in: MicroVertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world = in.inst_pos + in.position * in.inst_size;
    out.clip_position = camera.view_proj * vec4<f32>(world, 1.0);
    out.normal = in.normal;        // real per-face normal → 3D directional shading
    out.world_pos = world;
    out.tex_layer = in.tex_layer;  // per-vertex, baked from the source block
    out.uv = in.uv;
    out.light = in.inst_light;     // placement light (baked vertex light is FULL_BRIGHT)
    // Campaign N — placement sky light rides its own channel (night dimming).
    out.sky_light = in.inst_sky;
    return out;
}

@fragment
fn fs_plant(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(block_textures, block_sampler, in.uv, i32(in.tex_layer));
    // Alpha cutout — the cross billboard's transparent texels are dropped so
    // the plant reads as a stem, not a quad. (The chunk fs_main forces
    // alpha=1, which is fine for cubes but wrong for billboards.)
    if (tex_color.a < 0.5) {
        discard;
    }
    let sun_dir = normalize(camera.sun_dir.xyz);
    let brightness = camera.sun_dir.w;
    let sun_color = vec3<f32>(1.0, 0.98, 0.9) * brightness;
    let ambient = vec3<f32>(0.12, 0.13, 0.18) + vec3<f32>(0.28, 0.29, 0.32) * brightness;
    let ndotl = max(dot(in.normal, sun_dir), 0.0);
    // Campaign N review fix — `light` is BLOCK-only now (sky rides its own
    // channel), so combine with sky * sun.w exactly like fs_main; without this
    // every sky-lit plant rendered at the 0.08 floor in broad daylight.
    let block_light = min(max(max(in.light, in.sky_light * camera.sun_dir.w), 0.08), 1.0);
    let lit = tex_color.rgb * (ambient + sun_color * ndotl * 0.6) * block_light;

    let underwater = camera.camera_pos.w > 0.5;
    let dist = distance(in.world_pos, camera.camera_pos.xyz);
    var fog_start: f32;
    var fog_end: f32;
    var fog_color: vec3<f32>;
    if underwater {
        fog_start = 16.0;
        fog_end = 48.0;
        fog_color = vec3<f32>(0.1, 0.3, 0.6) * brightness;
    } else {
        fog_start = camera.fog.x;
        fog_end = camera.fog.y;
        fog_color = vec3<f32>(0.01 + 0.52 * brightness, 0.01 + 0.71 * brightness, 0.05 + 0.85 * brightness);
    }
    let fog_factor = clamp((dist - fog_start) / (fog_end - fog_start), 0.0, 1.0);
    var final_color = mix(lit, fog_color, fog_factor);
    if underwater {
        final_color = final_color * vec3<f32>(0.4, 0.6, 0.9);
    }
    return vec4<f32>(apply_user_brightness(final_color), 1.0);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample block texture
    let tex_color = textureSample(block_textures, block_sampler, in.uv, i32(in.tex_layer));

    // Dynamic sun light from uniform
    let sun_dir = normalize(camera.sun_dir.xyz);
    let brightness = camera.sun_dir.w;
    let sun_color = vec3<f32>(1.0, 0.98, 0.9) * brightness;
    let ambient = vec3<f32>(0.12, 0.13, 0.18) + vec3<f32>(0.28, 0.29, 0.32) * brightness;

    let ndotl = max(dot(in.normal, sun_dir), 0.0);
    // Spec 30 — block-light factor (0..1) from the BFS lighting pass.
    // Caves with no torches drop to MIN_AMBIENT so they don't go fully
    // black; sky-lit + torch-lit areas stay at full brightness.
    // P1 (2026-07-04): sky-light dims with time-of-day (sun_dir.w carries the
    // day/night brightness, night floor 0.15) while torch/block light doesn't —
    // nights get dark, torches finally matter. Non-terrain paths pass
    // sky_light 0 and are unchanged.
    let combined = max(in.light, in.sky_light * camera.sun_dir.w);
    let block_light = min(max(combined, 0.08), 1.0);
    let lit_normal = tex_color.rgb * (ambient + sun_color * ndotl * 0.6) * block_light;
    // Emissive sentinel: a vertex `light` > 1.5 marks a self-glowing part (e.g.
    // Satoshi's amulet, stamped with entity_model::SATOSHI_GLOW_LIGHT). Render it
    // mostly unlit so it glows even in a dim hut. All other geometry uses
    // light <= 1.0, so this branch is inert for everything else.
    let lit = mix(lit_normal, tex_color.rgb, step(1.5, in.light) * 0.85);

    // Underwater detection (packed in camera_pos.w)
    let underwater = camera.camera_pos.w > 0.5;

    // Distance fog — colour blends with sky brightness
    let dist = distance(in.world_pos, camera.camera_pos.xyz);
    var fog_start: f32;
    var fog_end: f32;
    var fog_color: vec3<f32>;
    if underwater {
        fog_start = 16.0;
        fog_end = 48.0;
        fog_color = vec3<f32>(0.1, 0.3, 0.6) * brightness;
    } else {
        fog_start = camera.fog.x;
        fog_end = camera.fog.y;
        // Fog matches sky colour (transitions with day/night)
        fog_color = vec3<f32>(0.01 + 0.52 * brightness, 0.01 + 0.71 * brightness, 0.05 + 0.85 * brightness);
    }
    let fog_factor = clamp((dist - fog_start) / (fog_end - fog_start), 0.0, 1.0);

    var final_color = mix(lit, fog_color, fog_factor);

    // Underwater color tint
    if underwater {
        final_color = final_color * vec3<f32>(0.4, 0.6, 0.9);
    }

    return vec4<f32>(apply_user_brightness(final_color), 1.0);
}

// Avatar fragment shader — identical lighting/fog/underwater to `fs_main`, but
// samples the dedicated `skin_texture` (group 2) instead of the block array,
// and alpha-cuts so the skin's transparent texels (e.g. the unused overlay
// layer) are dropped rather than drawn as opaque blocks. The avatar pipeline
// uses REPLACE blending, so the cutout is what gives the model clean edges.
@fragment
fn fs_avatar(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample the per-avatar skin texture array (mirrors the block-array idiom;
    // tex_layer selects this player's skin — 0 = local player, Phase 2 default).
    let tex_color = textureSample(skin_texture, skin_sampler, in.uv, i32(in.tex_layer));
    // Alpha cutout — drop transparent skin texels (no model matrix here, so we
    // discard rather than blend; REPLACE blending can't fade these out).
    if (tex_color.a < 0.5) {
        discard;
    }

    // Phase 3 — self-avatar fade-on-occlusion. The avatar pipeline repurposes the
    // per-vertex `light` channel as a fade alpha (1.0 = opaque; remote avatars and
    // the viewmodel arm always pass 1.0, so they never fade). Screen-door dither:
    // discard a fraction of fragments by a 4×4 Bayer threshold keyed to the pixel,
    // so a partially-faded body shows the scene through a fine stipple — this works
    // with the opaque REPLACE pipeline (no alpha blending, no depth sorting).
    // Render-only; the aim ray is untouched.
    let fade = in.light;
    if (fade < 0.999) {
        var bayer = array<f32, 16>(
             0.0 / 16.0,  8.0 / 16.0,  2.0 / 16.0, 10.0 / 16.0,
            12.0 / 16.0,  4.0 / 16.0, 14.0 / 16.0,  6.0 / 16.0,
             3.0 / 16.0, 11.0 / 16.0,  1.0 / 16.0,  9.0 / 16.0,
            15.0 / 16.0,  7.0 / 16.0, 13.0 / 16.0,  5.0 / 16.0
        );
        let px = u32(in.clip_position.x) % 4u;
        let py = u32(in.clip_position.y) % 4u;
        // Strict `<` so even at fade→0 the lowest-threshold pixel (bayer 0) survives
        // (a faint stipple) rather than the body vanishing entirely.
        if (fade < bayer[py * 4u + px]) {
            discard;
        }
    }

    // Dynamic sun light from uniform
    let sun_dir = normalize(camera.sun_dir.xyz);
    let brightness = camera.sun_dir.w;
    let sun_color = vec3<f32>(1.0, 0.98, 0.9) * brightness;
    let ambient = vec3<f32>(0.12, 0.13, 0.18) + vec3<f32>(0.28, 0.29, 0.32) * brightness;

    let ndotl = max(dot(in.normal, sun_dir), 0.0);
    // Campaign N — the `light` channel carries the Phase-3 fade alpha
    // (consumed by the dither above); `sky_light` carries a CPU-resolved
    // combined world-light scalar (`entity_model::set_avatar_light`; feeders
    // default to 1.0 via `push_skin_quad`). Same 0.08 floor as terrain.
    let block_light = min(max(in.sky_light, 0.08), 1.0);
    let lit = tex_color.rgb * (ambient + sun_color * ndotl * 0.6) * block_light;

    // Underwater detection (packed in camera_pos.w)
    let underwater = camera.camera_pos.w > 0.5;

    // Distance fog — colour blends with sky brightness
    let dist = distance(in.world_pos, camera.camera_pos.xyz);
    var fog_start: f32;
    var fog_end: f32;
    var fog_color: vec3<f32>;
    if underwater {
        fog_start = 16.0;
        fog_end = 48.0;
        fog_color = vec3<f32>(0.1, 0.3, 0.6) * brightness;
    } else {
        fog_start = camera.fog.x;
        fog_end = camera.fog.y;
        // Fog matches sky colour (transitions with day/night)
        fog_color = vec3<f32>(0.01 + 0.52 * brightness, 0.01 + 0.71 * brightness, 0.05 + 0.85 * brightness);
    }
    let fog_factor = clamp((dist - fog_start) / (fog_end - fog_start), 0.0, 1.0);

    var final_color = mix(lit, fog_color, fog_factor);

    // Underwater color tint
    if underwater {
        final_color = final_color * vec3<f32>(0.4, 0.6, 0.9);
    }

    return vec4<f32>(apply_user_brightness(final_color), 1.0);
}

// Block-break crack overlay (Spec 05 §2.2). Samples a crack-stage layer from
// the block texture array and returns it unlit — the texture carries its own
// alpha, so only the crack pixels darken the block face. No lighting or fog, so
// cracks read identically in caves and daylight.
@fragment
fn fs_crack(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(block_textures, block_sampler, in.uv, i32(in.tex_layer));
}

// Owner-inbox #1/2/3 — wallpaper face-overlay decal. Samples the wallpaper
// layer, drops fully-transparent texels (alpha cutout, like fs_plant) so future
// translucent/bordered wallpaper works, and multiplies by the baked vertex
// light so the paper darkens in dim rooms instead of glowing.
@fragment
fn fs_decal(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(block_textures, block_sampler, in.uv, i32(in.tex_layer));
    if (c.a < 0.5) {
        discard;
    }
    // Campaign N review fix — `light` is BLOCK-only; combine with the sky
    // channel like fs_main so sun-lit wallpaper/blueprints don't go black.
    let combined = max(in.light, in.sky_light * camera.sun_dir.w);
    return vec4<f32>(c.rgb * min(max(combined, 0.08), 1.0), c.a);
}

// The Gallery — hung painting. Samples the dedicated hi-res texture and lights
// it like a piece under bright gallery lighting: a high ambient floor so the art
// reads true even on a wall the sun doesn't hit, plus a gentle directional term
// so it isn't perfectly flat. Shares the terrain fog so distant art fades with
// the world. Opaque (the quad IS the frame).
@fragment
fn fs_painting(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(painting_tex, painting_samp, in.uv);
    let sun_dir = normalize(camera.sun_dir.xyz);
    let brightness = camera.sun_dir.w;
    let sun_color = vec3<f32>(1.0, 0.98, 0.9) * brightness;
    // Bright, near-flat gallery lighting — high ambient so colours stay true.
    let ambient = vec3<f32>(0.45, 0.46, 0.48) + vec3<f32>(0.4, 0.41, 0.42) * brightness;
    let ndotl = max(dot(normalize(in.normal), sun_dir), 0.0);
    let lit = tex_color.rgb * (ambient + sun_color * ndotl * 0.25);

    let dist = distance(in.world_pos, camera.camera_pos.xyz);
    let fog_start = camera.fog.x;
    let fog_end = camera.fog.y;
    let fog_color = vec3<f32>(0.01 + 0.52 * brightness, 0.01 + 0.71 * brightness, 0.05 + 0.85 * brightness);
    let fog_factor = clamp((dist - fog_start) / (fog_end - fog_start), 0.0, 1.0);
    let final_color = mix(lit, fog_color, fog_factor);
    return vec4<f32>(apply_user_brightness(final_color), 1.0);
}

// Exhibits — standing billboard. Same gallery lighting as `fs_painting`, but a
// hard alpha cut-out (like `fs_plant` / `fs_avatar`): transparent texels are
// discarded so a PNG cut-out reads as the object, not a rectangle. Returns the
// texel alpha so the surviving edge stays crisp.
@fragment
fn fs_painting_alpha(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(painting_tex, painting_samp, in.uv);
    if (tex_color.a < 0.5) {
        discard;
    }
    let sun_dir = normalize(camera.sun_dir.xyz);
    let brightness = camera.sun_dir.w;
    let sun_color = vec3<f32>(1.0, 0.98, 0.9) * brightness;
    let ambient = vec3<f32>(0.45, 0.46, 0.48) + vec3<f32>(0.4, 0.41, 0.42) * brightness;
    let ndotl = max(dot(normalize(in.normal), sun_dir), 0.0);
    let lit = tex_color.rgb * (ambient + sun_color * ndotl * 0.25);

    let dist = distance(in.world_pos, camera.camera_pos.xyz);
    let fog_start = camera.fog.x;
    let fog_end = camera.fog.y;
    let fog_color = vec3<f32>(0.01 + 0.52 * brightness, 0.01 + 0.71 * brightness, 0.05 + 0.85 * brightness);
    let fog_factor = clamp((dist - fog_start) / (fog_end - fog_start), 0.0, 1.0);
    let final_color = mix(lit, fog_color, fog_factor);
    return vec4<f32>(apply_user_brightness(final_color), tex_color.a);
}

@fragment
fn fs_water(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(block_textures, block_sampler, in.uv, i32(in.tex_layer));

    let brightness = camera.sun_dir.w;
    let water_tint = vec3<f32>(0.3, 0.5, 0.9) * (0.3 + 0.7 * brightness);
    // Spec 30 — water also darkens in caves; reuse the same block-light.
    let block_light = max(in.light, 0.08);
    let lit = tex_color.rgb * water_tint * block_light;

    // Fog
    let dist = distance(in.world_pos, camera.camera_pos.xyz);
    let fog_start = camera.fog.z;
    let fog_end = camera.fog.w;
    let fog_color = vec3<f32>(0.3, 0.5, 0.7) * brightness;
    let fog_factor = clamp((dist - fog_start) / (fog_end - fog_start), 0.0, 1.0);

    let final_color = mix(lit, fog_color, fog_factor);
    return vec4<f32>(apply_user_brightness(final_color), 0.65);
}

// Transparent-solid fragment (#131) — glass + other solid+transparent blocks.
// Same lighting + fog as fs_main, but it OUTPUTS THE TEXTURE'S OWN ALPHA (so
// glass's translucent edges read as glass) and runs in the alpha-blended,
// depth-read-only transparent pass. The near-zero-alpha discard means a future
// cutout-alpha leaf texture renders as see-through holes ("fancy" leaves) with
// no extra plumbing.
@fragment
fn fs_transparent(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(block_textures, block_sampler, in.uv, i32(in.tex_layer));
    if (tex_color.a < 0.04) { discard; }

    let sun_dir = normalize(camera.sun_dir.xyz);
    let brightness = camera.sun_dir.w;
    let sun_color = vec3<f32>(1.0, 0.98, 0.9) * brightness;
    let ambient = vec3<f32>(0.12, 0.13, 0.18) + vec3<f32>(0.28, 0.29, 0.32) * brightness;
    let ndotl = max(dot(in.normal, sun_dir), 0.0);
    let block_light = max(in.light, 0.08);
    let lit = tex_color.rgb * (ambient + sun_color * ndotl * 0.6) * block_light;

    let dist = distance(in.world_pos, camera.camera_pos.xyz);
    let fog_start = camera.fog.x;
    let fog_end = camera.fog.y;
    let fog_color = vec3<f32>(0.01 + 0.52 * brightness, 0.01 + 0.71 * brightness, 0.05 + 0.85 * brightness);
    let fog_factor = clamp((dist - fog_start) / (fog_end - fog_start), 0.0, 1.0);

    let final_color = mix(lit, fog_color, fog_factor);
    return vec4<f32>(apply_user_brightness(final_color), tex_color.a);
}

// --- Spec 39 Phase 5: render-scale blit ---
// --- Particle billboards (2026-07-05) ---
// Vertex buffer 0: the shared unit quad (PlantGeoVertex — local xy in
// [-0.5, 0.5], z 0, uv). Vertex buffer 1 (per instance): world pos + size
// (one vec4), colour, texture layer. The quad is expanded toward the camera
// with cam_right/cam_up, so every particle faces the viewport's own eye.
struct ParticleVertexInput {
    @location(0) local: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) pos_size: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) tex_layer: u32,
};

struct ParticleOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) tex_layer: u32,
};

@vertex
fn vs_particle(in: ParticleVertexInput) -> ParticleOutput {
    var out: ParticleOutput;
    let centre = in.pos_size.xyz;
    let size = in.pos_size.w;
    let world = centre
        + camera.cam_right.xyz * (in.local.x * size * 2.0)
        + camera.cam_up.xyz * (in.local.y * size * 2.0);
    out.clip_position = camera.view_proj * vec4<f32>(world, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    out.tex_layer = in.tex_layer;
    return out;
}

@fragment
fn fs_particle(in: ParticleOutput) -> @location(0) vec4<f32> {
    let tex = textureSample(block_textures, block_sampler, in.uv, i32(in.tex_layer));
    // Night factor: particles dim with the sky so rain/smoke doesn't glow in
    // the dark (sun_dir.w = sky brightness; floor keeps embers readable).
    let night = mix(0.35, 1.0, camera.sun_dir.w);
    let a = tex.a * in.color.a;
    if (a < 0.01) {
        discard;
    }
    return vec4<f32>(tex.rgb * in.color.rgb * night, a);
}

// Upscales the offscreen world target (rendered at render_scale × resolution)
// to the full-resolution surface. Fullscreen triangle, no vertex buffer.
struct BlitOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_blit(@builtin(vertex_index) vid: u32) -> BlitOut {
    var out: BlitOut;
    // Oversized triangle covering the screen: uv (0,0),(2,0),(0,2).
    let x = f32((vid << 1u) & 2u);
    let y = f32(vid & 2u);
    out.uv = vec2<f32>(x, y);
    out.clip = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

@group(0) @binding(0) var blit_tex: texture_2d<f32>;
@group(0) @binding(1) var blit_samp: sampler;

@fragment
fn fs_blit(in: BlitOut) -> @location(0) vec4<f32> {
    return textureSample(blit_tex, blit_samp, in.uv);
}

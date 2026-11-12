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
    // x = user brightness control; yzw = live sky horizon RGB.
    params: vec4<f32>,
    // Particle billboarding (2026-07-05): the camera's world-space right/up
    // (xyz; w unused). Per-player, so split-screen billboards face each eye.
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    // Living-world bloom (spec 2026-11-08 §2). bloom_origin: xy = world x/z of
    // the grid's min corner, z = cell size (0 = grid off, so nothing is graded
    // and nothing sways), w = shader clock in seconds. bloom_grid: 16 x 16
    // region levels, row-major in z then x, four to a vec4.
    bloom_origin: vec4<f32>,
    bloom_grid: array<vec4<f32>, 64>,
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
    let lit = max(c, vec3<f32>(0.0));
    // Neutral (the default) and unset uniforms skip the per-fragment pow. The
    // branch is on a uniform, so it is coherent across the whole draw.
    if (abs(x - 1.0) < 0.0001 || x <= 0.001) {
        return lit;
    }
    return pow(lit, vec3<f32>(1.0 / x));
}

// ── Living-world bloom: colour grade and grass sway (spec §2) ────────────────
// CPU mirror + tests: `bloom/present.rs` (`grade`, `sample_level`, `sway_amplitude`).
fn bloom_cell(i: i32, j: i32) -> f32 {
    let k = u32(clamp(j, 0, 15) * 16 + clamp(i, 0, 15));
    return camera.bloom_grid[k >> 2u][k & 3u];
}

// The level in the camera's own cell.
fn bloom_camera_level() -> f32 {
    let g = floor((camera.camera_pos.xz - camera.bloom_origin.xy) / max(camera.bloom_origin.z, 0.0001));
    return bloom_cell(i32(g.x), i32(g.y));
}

// Bilinear level at a world x/z (cell centres are the sample points). Outside
// the grid, or with the grid off, the camera-cell level (0.6 when off).
fn bloom_level_at(xz: vec2<f32>) -> f32 {
    let cell = camera.bloom_origin.z;
    if (cell <= 0.0) {
        return 0.6;
    }
    let g = (xz - camera.bloom_origin.xy) / cell;
    if (g.x < 0.0 || g.y < 0.0 || g.x >= 16.0 || g.y >= 16.0) {
        return bloom_camera_level();
    }
    let u = g - vec2<f32>(0.5);
    let f = floor(u);
    let t = u - f;
    let i = i32(f.x);
    let j = i32(f.y);
    let a = mix(bloom_cell(i, j), bloom_cell(i + 1, j), t.x);
    let b = mix(bloom_cell(i, j + 1), bloom_cell(i + 1, j + 1), t.x);
    return mix(a, b, t.y);
}

// >= 0.45 and <= 0.75: unchanged. Below 0.45: desaturate toward luminance (15%
// saturation left at level 0) with a slight cool cast. Above 0.75: up to +15%
// saturation with a slight warm lift.
fn bloom_grade_level(c: vec3<f32>, level: f32) -> vec3<f32> {
    let lum = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    if (level < 0.45) {
        let t = clamp(1.0 - level / 0.45, 0.0, 1.0);
        let g = mix(vec3<f32>(lum), c, 1.0 - 0.85 * t);
        return max(g * mix(vec3<f32>(1.0), vec3<f32>(0.95, 0.98, 1.04), t), vec3<f32>(0.0));
    }
    if (level > 0.75) {
        let t = clamp((level - 0.75) / 0.25, 0.0, 1.0);
        let g = mix(vec3<f32>(lum), c, 1.0 + 0.15 * t);
        return max(g * mix(vec3<f32>(1.0), vec3<f32>(1.04, 1.015, 0.95), t), vec3<f32>(0.0));
    }
    return c;
}

fn bloom_grade(c: vec3<f32>, world_xz: vec2<f32>) -> vec3<f32> {
    if (camera.bloom_origin.z <= 0.0) {
        return c;
    }
    return bloom_grade_level(c, bloom_level_at(world_xz));
}

// Sky and fog horizon: graded by the level in the camera's own cell.
fn bloom_grade_camera(c: vec3<f32>) -> vec3<f32> {
    if (camera.bloom_origin.z <= 0.0) {
        return c;
    }
    return bloom_grade_level(c, bloom_camera_level());
}

// Grass sway amplitude (blocks at the plant's tip) by local level: none at or
// below 0.3, gentle (0.035) at 0.6, a bit more (0.06) at 1.0.
fn bloom_sway_amplitude(level: f32) -> f32 {
    return 0.035 * smoothstep(0.3, 0.6, level) + 0.025 * smoothstep(0.6, 1.0, level);
}

// The renderer supplies the live horizon colour, including weather/lightning.
// Standalone uniforms with no supplied colour retain the day/night fallback.
fn sky_horizon() -> vec3<f32> {
    let b = camera.sun_dir.w;
    let fallback = vec3<f32>(0.01 + 0.52 * b, 0.01 + 0.71 * b, 0.05 + 0.85 * b);
    let base = select(fallback, camera.params.yzw, any(camera.params.yzw != vec3<f32>(0.0)));
    return bloom_grade_camera(base);
}

// ── One light rule (Spec 03 §4.7) ────────────────────────────────────────────
// CPU twin + tests: `world_light.rs` (the drift guard there checks these
// constants). A surface has two light channels, block (torches, lava, lamps)
// and sky, each 0..1, and they light it in two different ways:
//   sky term   = sky_shade(n) * clamp(sky * sun.w, LIGHT_FLOOR, 1)
//   block term = BLOCK_LIGHT_COLOUR * min(block, 1) * face_shade(n)
//   light      = per-channel max of the two
// The block term has no time input: a torch lights a room the same at noon and
// midnight. (Until 2026-11-12 the channels were merged first and the merged
// value scaled by sky_shade, so torchlight dimmed with the sun.) Branch-free.
const SKY_AMBIENT_NIGHT: vec3<f32> = vec3<f32>(0.12, 0.13, 0.18);
const SKY_AMBIENT_DAY: vec3<f32> = vec3<f32>(0.28, 0.29, 0.32);
const SUN_COLOUR: vec3<f32> = vec3<f32>(1.0, 0.98, 0.9);
const SUN_WEIGHT: f32 = 0.6;
const LIGHT_FLOOR: f32 = 0.08;
const BLOCK_LIGHT_COLOUR: vec3<f32> = vec3<f32>(1.0, 0.9, 0.7);
const FACE_SHADE_TOP: f32 = 1.0;
const FACE_SHADE_BOTTOM: f32 = 0.5;
const FACE_SHADE_Z: f32 = 0.8;
const FACE_SHADE_X: f32 = 0.6;
// Avatar vertices keep the skin layer in the low 16 bits of tex_layer and the
// block-light level (0..15) in bits 20..23 (their `light` is the fade alpha).
const AVATAR_LAYER_MASK: u32 = 0xFFFFu;
const AVATAR_BLOCK_SHIFT: u32 = 20u;
const AVATAR_BLOCK_MASK: u32 = 0xFu;

// The sun-and-ambient shade of a face (what sky light is multiplied by).
fn sky_shade(n: vec3<f32>) -> vec3<f32> {
    let b = camera.sun_dir.w;
    let ambient = SKY_AMBIENT_NIGHT + SKY_AMBIENT_DAY * b;
    let ndotl = max(dot(n, normalize(camera.sun_dir.xyz)), 0.0);
    return ambient + SUN_COLOUR * b * ndotl * SUN_WEIGHT;
}

// `sky_level` is already scaled by the day's brightness (sky * sun.w).
fn sky_term(n: vec3<f32>, sky_level: f32) -> vec3<f32> {
    return sky_shade(n) * min(max(sky_level, LIGHT_FLOOR), 1.0);
}

// Minecraft's fixed face shade for block light: top 1.0, bottom 0.5, north and
// south 0.8, east and west 0.6. Other normals mix by their squared components;
// a zero normal is unshaded. Both select arms are evaluated, so the divisor is
// kept away from zero.
fn face_shade(n: vec3<f32>) -> f32 {
    let n2 = n * n;
    let len2 = n2.x + n2.y + n2.z;
    let y_shade = select(FACE_SHADE_BOTTOM, FACE_SHADE_TOP, n.y > 0.0);
    let weighted = n2.x * FACE_SHADE_X + n2.y * y_shade + n2.z * FACE_SHADE_Z;
    return select(1.0, weighted / max(len2, 0.000001), len2 > 0.000001);
}

fn block_term(n: vec3<f32>, block_level: f32) -> vec3<f32> {
    return BLOCK_LIGHT_COLOUR * (min(block_level, 1.0) * face_shade(n));
}

// The rule for a surface carrying the two raw channels (`light` = block,
// `sky_light` = sky): what its texture colour is multiplied by.
fn world_light(n: vec3<f32>, block_level: f32, sky_level: f32) -> vec3<f32> {
    return max(sky_term(n, sky_level * camera.sun_dir.w), block_term(n, block_level));
}

struct SkyOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) vertex: u32) -> SkyOutput {
    var out: SkyOutput;
    let p = vec2<f32>(f32((vertex << 1u) & 2u), f32(vertex & 2u)) * 2.0 - 1.0;
    out.clip = vec4<f32>(p, 1.0, 1.0);
    out.ndc = p;
    return out;
}

@fragment
fn fs_sky(in: SkyOutput) -> @location(0) vec4<f32> {
    // Perspective VP rows encode scaled right/up and unit forward. Recover
    // the world ray without trusting billboard axes or adding a matrix upload.
    let m = camera.view_proj;
    let right = vec3<f32>(m[0][0], m[1][0], m[2][0]);
    let up = vec3<f32>(m[0][1], m[1][1], m[2][1]);
    let forward = vec3<f32>(m[0][3], m[1][3], m[2][3]);
    let ray = forward + in.ndc.x * right / max(dot(right, right), 0.0001)
                      + in.ndc.y * up / max(dot(up, up), 0.0001);
    let altitude = ray.y / max(length(ray), 0.0001);
    let horizon = sky_horizon();
    // Keep the horizon continuous with land fog, with a deeper blue overhead.
    // Near-white lightning colours remain white throughout the sky.
    let white = smoothstep(0.8, 0.98, min(horizon.r, min(horizon.g, horizon.b)));
    let zenith = horizon * mix(vec3<f32>(0.35, 0.55, 0.86), vec3<f32>(1.0), white);
    let colour = mix(horizon, zenith, smoothstep(0.0, 0.85, altitude));
    return vec4<f32>(apply_user_brightness(colour), 1.0);
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
    // Block-light channel (torches, lava, lamps), 0..1, or the > 1.5 emissive
    // sentinel. On the avatar pipeline: the fade alpha instead.
    @location(4) light: f32,
    // P1 (2026-07-04): separate sky-light channel, scaled by time-of-day in the
    // light rule (`world_light`) so nights darken sky-lit surfaces while
    // torches keep working. Every feeder passes real (block, sky) channels: a
    // `light` with `sky_light` 0 renders as torchlight (warm, unshaded, as
    // bright at midnight as at noon). On the avatar pipeline: sky already
    // scaled by the day's brightness.
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
    var world = in.inst_pos + in.local * in.inst_size;
    // Living-world grass sway: the tip (local.y = 1) leans, the root stays put.
    // Off with the grid off (headless / menu paths). Phase from the plant's
    // own position so a meadow ripples rather than nodding in step.
    if (camera.bloom_origin.z > 0.0) {
        let amp = bloom_sway_amplitude(bloom_level_at(in.inst_pos.xz));
        let t = camera.bloom_origin.w;
        let ph = in.inst_pos.x * 0.7 + in.inst_pos.z * 0.45;
        let lean = in.local.y * in.local.y * amp * in.inst_size.y;
        world.x += lean * sin(t * 1.7 + ph);
        world.z += lean * sin(t * 1.3 + ph * 1.3 + 1.7);
    }
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
    let brightness = camera.sun_dir.w;
    // The one light rule (Spec 03 §4.7), like fs_main. `light` is the block
    // channel and `sky_light` the sky channel; the normal is straight up
    // (vs_plant), so the block term is unshaded.
    let lit = bloom_grade(tex_color.rgb * world_light(in.normal, in.light, in.sky_light), in.world_pos.xz);

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
        fog_color = sky_horizon();
    }
    let fog_factor = clamp((dist - fog_start) / (fog_end - fog_start), 0.0, 1.0);
    var final_color = mix(lit, fog_color, fog_factor);
    if underwater {
        final_color = final_color * vec3<f32>(0.4, 0.6, 0.9);
    }
    return vec4<f32>(apply_user_brightness(final_color), 1.0);
}

// "Leaves: Fast" (Perf B2): the mesher sets bit 16 of `tex_layer` on a leaf face
// it draws in this opaque pass (`mesh::LEAF_FILL_FLAG`). The leaf texture has
// cutout holes (alpha 0) that the transparent pass would discard; here there is
// nothing behind them, so a hole shows a darkened leaf colour instead of a
// stray texel (the texture's own rgb under alpha 0 is not guaranteed to be leaf
// coloured). Uses the texture's smallest mip (the alpha-weighted average leaf
// colour when mipmaps are on, else the texel itself); a near-black result falls
// back to a dark leaf green so a hole is never a black spot.
const LEAF_FILL_FLAG: u32 = 0x10000u;
const LEAF_LAYER_MASK: u32 = 0xFFFFu;

// Item sprites (`item_model`): the mesher and the item builders set bit 17 of
// tex_layer (`mesh::CUTOUT_FLAG`) on every face of a framed, held or dropped
// sprite item. fs_main drops a texel whose full-size alpha is below 0.5, so a
// flat item shows its silhouette, not the square it is painted on.
const CUTOUT_FLAG: u32 = 0x20000u;

fn fast_leaf_fill(uv: vec2<f32>, layer: i32) -> vec3<f32> {
    let avg = textureSampleLevel(block_textures, block_sampler, uv, layer, 4.0).rgb;
    let dark = max(avg.r, max(avg.g, avg.b)) < 0.02;
    return select(avg, vec3<f32>(0.05, 0.14, 0.04), dark) * 0.6;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample block texture
    let layer = i32(in.tex_layer & LEAF_LAYER_MASK);
    let tex_sample = textureSample(block_textures, block_sampler, in.uv, layer);
    var tex_color = tex_sample;
    if ((in.tex_layer & LEAF_FILL_FLAG) != 0u && tex_sample.a < 0.5) {
        tex_color = vec4<f32>(fast_leaf_fill(in.uv, layer), 1.0);
    }
    // An item sprite (CUTOUT_FLAG) drops its transparent texels. The test reads
    // the full-size texel (mip level 0): a two-texel torch handle averages to
    // half alpha one mip down and would vanish at a distance.
    if ((in.tex_layer & CUTOUT_FLAG) != 0u
        && textureSampleLevel(block_textures, block_sampler, in.uv, layer, 0.0).a < 0.5) {
        discard;
    }

    let brightness = camera.sun_dir.w;
    // Spec 30 / Spec 03 §4.7 — the one light rule. `light` is the block
    // channel (torches, lava, lamps; BFS-propagated), `sky_light` the sky
    // channel. Sky light dims with the day (sun.w) and is shaded by the sun;
    // block light keeps its warm colour and a fixed per-face shade at any hour.
    // A sealed cave with neither drops to the LIGHT_FLOOR, not to black.
    let lit_normal = tex_color.rgb * world_light(in.normal, in.light, in.sky_light);
    // Emissive sentinel: a vertex `light` > 1.5 marks a self-glowing part (a
    // torch's own faces, Satoshi's amulet, the GlowSquid; `Vertex::EMISSIVE`).
    // Render it mostly unlit so it glows even in a dim hut. All other geometry
    // uses light <= 1.0, so this mix is inert for everything else.
    let lit = bloom_grade(mix(lit_normal, tex_color.rgb, step(1.5, in.light) * 0.85), in.world_pos.xz);

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
        fog_color = sky_horizon();
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
    // the low 16 bits of tex_layer select this player's skin — 0 = local
    // player, Phase 2 default; bits 20..23 carry the block-light level).
    let tex_color = textureSample(skin_texture, skin_sampler, in.uv, i32(in.tex_layer & AVATAR_LAYER_MASK));
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

    let brightness = camera.sun_dir.w;
    // The one light rule (Spec 03 §4.7), with the channels where an avatar
    // keeps them: the `light` channel is the Phase-3 fade alpha (consumed by
    // the dither above), so `sky_light` carries sky ALREADY scaled by the day's
    // brightness on the CPU (not multiplied by sun.w again here) and the
    // block-light level rides in bits 20..23 of tex_layer
    // (`entity_model::set_avatar_light`). `push_skin_quad`'s defaults (sky 1.0,
    // no block bits) give the sky term alone, as before.
    let block_level = f32((in.tex_layer >> AVATAR_BLOCK_SHIFT) & AVATAR_BLOCK_MASK) / 15.0;
    let shade = max(sky_term(in.normal, in.sky_light), block_term(in.normal, block_level));
    let lit = tex_color.rgb * shade;

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
        fog_color = sky_horizon();
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
    // The one light rule (Spec 03 §4.7), the same as the wall under it. Since
    // 2026-11-12 the paper also takes the sun-and-ambient face shade, so in
    // daylight it darkens to match its wall instead of reading brighter.
    return vec4<f32>(c.rgb * world_light(in.normal, in.light, in.sky_light), c.a);
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
    let fog_color = sky_horizon();
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
    let fog_color = sky_horizon();
    let fog_factor = clamp((dist - fog_start) / (fog_end - fog_start), 0.0, 1.0);
    let final_color = mix(lit, fog_color, fog_factor);
    return vec4<f32>(apply_user_brightness(final_color), tex_color.a);
}

@fragment
fn fs_water(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(block_textures, block_sampler, in.uv, i32(in.tex_layer));

    let brightness = camera.sun_dir.w;
    let water_tint = vec3<f32>(0.3, 0.5, 0.9);
    // Spec 30 / Spec 03 §4.7 — water darkens in caves. Its sky half is the
    // floor, dimmed with the day as before ((0.3 + 0.7 * sun.w) * LIGHT_FLOOR);
    // it still reads no sky channel (a known gap, Spec 03 §4.7). Its block half
    // is the shared block term, with no time input, so a torch lights water the
    // same at noon and midnight.
    let sky_floor = (0.3 + 0.7 * brightness) * LIGHT_FLOOR;
    let shade = max(vec3<f32>(sky_floor), block_term(in.normal, in.light));
    let lit = bloom_grade(tex_color.rgb * water_tint * shade, in.world_pos.xz);

    // Fog
    let dist = distance(in.world_pos, camera.camera_pos.xyz);
    let fog_start = camera.fog.z;
    let fog_end = camera.fog.w;
    let fog_color = bloom_grade_camera(vec3<f32>(0.3, 0.5, 0.7) * brightness);
    let fog_factor = clamp((dist - fog_start) / (fog_end - fog_start), 0.0, 1.0);

    let final_color = mix(lit, fog_color, fog_factor);
    return vec4<f32>(apply_user_brightness(final_color), 0.65);
}

// Only fully opaque texels write depth. Holes and partly translucent glass
// retain the previous terrain depth, so water behind them stays visible.
@fragment
fn fs_solid_depth(in: VertexOutput) -> @location(0) vec4<f32> {
    let alpha = textureSample(block_textures, block_sampler, in.uv, i32(in.tex_layer)).a;
    if (alpha < 0.9999) { discard; }
    return vec4<f32>(0.0);
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

    // Solid transparent surfaces receive the same two channels as terrain and
    // the same light rule (Spec 03 §4.7).
    let lit = bloom_grade(tex_color.rgb * world_light(in.normal, in.light, in.sky_light), in.world_pos.xz);

    let dist = distance(in.world_pos, camera.camera_pos.xyz);
    let fog_start = camera.fog.x;
    let fog_end = camera.fog.y;
    let fog_color = sky_horizon();
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

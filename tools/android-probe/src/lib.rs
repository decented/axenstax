//! Axe'n'Stax Android GPU probe — the decisive spike measurement.
//!
//! ## The question
//!
//! The block-texture atlas allocates ONE array layer per registered texture:
//! `texture_gen::texture_count()` == **506** today (459 `PRE_TINT_LAYER_COUNT`
//! + 47 species-tint layers), against a 512 pre-flight floor. Vulkan's spec
//! minimum for `maxImageArrayLayers` is only **256**. Desktop GPUs report 2048;
//! plenty of mobile drivers do too, but it is NOT guaranteed, and a device that
//! caps at 256 physically cannot hold the atlas.
//!
//! If Android phones report 256, an APK port is a renderer rework (split atlas
//! or layer paging), not a port. That is a weeks-vs-months fork, so it is worth
//! answering with ~150 lines before touching the 200k-line engine.
//!
//! ## Why this doesn't just read the limit
//!
//! Reading `adapter.limits().max_texture_array_layers` is necessary but NOT
//! sufficient — a driver can advertise a limit it then fails to honour for a
//! specific format. So the probe also performs the **real allocation**: a
//! 16×16×506 `Rgba8UnormSrgb` array, exactly the shape the engine builds. If
//! that succeeds, the atlas fits on this device. That is proof, not inference.
//!
//! ## Reading the result
//!
//!   adb logcat -s axeprobe:V
//!
//! Look for the `VERDICT:` lines.

use android_activity::{AndroidApp, MainEvent, PollEvent};

/// Layers the engine's block atlas needs. Keep in sync with
/// `texture_gen::texture_count()` — see that function and the
/// `texture_count_stays_under_webgpu_check_floor` test.
const REQUIRED_LAYERS: u32 = 506;

/// The JS pre-flight floor the web build gates on
/// (`tools/sites/game/static/webgpu-check.js` REQUIRED_TEXTURE_ARRAY_LAYERS).
/// Reported for context: clearing 506 but not 512 means the atlas fits today
/// with no headroom for the next texture wave.
const WEB_FLOOR: u32 = 512;

#[no_mangle]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("axeprobe"),
    );

    log::info!("=== Axe'n'Stax Android GPU probe ===");
    log::info!("engine needs {REQUIRED_LAYERS} texture array layers (web floor {WEB_FLOOR})");

    // Catch a panic in the probe so the activity still reports rather than
    // vanishing — a silent process death on a spike tells us nothing.
    let result = std::panic::catch_unwind(run_probe);
    if result.is_err() {
        log::error!("VERDICT: PROBE PANICKED — see the backtrace above");
    }

    // NativeActivity kills the process the moment android_main returns, which
    // can truncate logcat. Idle instead so the output is reliably flushed and
    // the device stays on the (blank) activity until dismissed.
    loop {
        let mut exit = false;
        app.poll_events(Some(std::time::Duration::from_millis(500)), |event| {
            if let PollEvent::Main(MainEvent::Destroy) = event {
                exit = true;
            }
        });
        if exit {
            log::info!("probe activity destroyed; exiting");
            return;
        }
    }
}

fn run_probe() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });

    // Enumerate everything first — a device often exposes both a Vulkan and a
    // GLES adapter with DIFFERENT limits, and the engine's Backends::all() would
    // pick Vulkan. Logging both shows how much headroom the fallback has.
    // wgpu 29: enumerate_adapters is async (it returns a Future, not a Vec).
    let adapters = pollster::block_on(
        instance.enumerate_adapters(wgpu::Backends::VULKAN | wgpu::Backends::GL),
    );
    log::info!("found {} adapter(s)", adapters.len());
    for (i, adapter) in adapters.iter().enumerate() {
        let info = adapter.get_info();
        let limits = adapter.limits();
        log::info!(
            "adapter[{i}]: {:?} backend={:?} type={:?} driver={:?} {:?}",
            info.name, info.backend, info.device_type, info.driver, info.driver_info
        );
        log::info!(
            "adapter[{i}]: max_texture_array_layers={} max_texture_dimension_2d={}",
            limits.max_texture_array_layers, limits.max_texture_dimension_2d
        );
    }

    // Now the adapter the engine would actually choose.
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no suitable GPU adapter on this device");

    let info = adapter.get_info();
    let advertised = adapter.limits().max_texture_array_layers;
    log::info!("CHOSEN adapter: {:?} ({:?})", info.name, info.backend);
    log::info!("VERDICT: advertised max_texture_array_layers = {advertised}");

    if advertised < REQUIRED_LAYERS {
        log::error!(
            "VERDICT: FAIL — device advertises {advertised} layers, engine needs {REQUIRED_LAYERS}. \
             The block atlas CANNOT fit as designed; an APK port needs a renderer rework \
             (split atlas or layer paging) before it can boot."
        );
        return;
    }

    // Mirror the engine's request_device: lift the limit to the adapter's max,
    // exactly as renderer.rs does, so a failure here is a failure there.
    let (device, _queue) = match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("axeprobe_device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits {
            max_texture_array_layers: advertised,
            ..wgpu::Limits::default()
        },
        ..Default::default()
    })) {
        Ok(pair) => pair,
        Err(e) => {
            log::error!("VERDICT: FAIL — request_device rejected the lifted limit: {e:?}");
            return;
        }
    };

    // Install an error scope so a validation failure is REPORTED rather than
    // taking the process down — we want a verdict either way.
    //
    // wgpu 29 made this RAII: push_error_scope returns a #[must_use] guard and
    // the errors are collected by consuming it with `.pop()` (there is no
    // `device.pop_error_scope()` any more).
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

    // The real thing: the exact texture the engine builds.
    let _atlas = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("axeprobe_block_textures"),
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: REQUIRED_LAYERS,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    match pollster::block_on(scope.pop()) {
        Some(err) => log::error!(
            "VERDICT: FAIL — advertised {advertised} layers but allocating the real \
             16x16x{REQUIRED_LAYERS} Rgba8UnormSrgb atlas was REJECTED: {err:?}"
        ),
        None => {
            log::info!(
                "VERDICT: PASS — allocated the real 16x16x{REQUIRED_LAYERS} atlas on {:?} ({:?}). \
                 The engine's block atlas fits this device.",
                info.name, info.backend
            );
            if advertised < WEB_FLOOR {
                log::warn!(
                    "VERDICT: NOTE — {advertised} clears today's {REQUIRED_LAYERS} but is under \
                     the {WEB_FLOOR} web floor, so there is little headroom for the next \
                     texture wave on this device."
                );
            }
        }
    }
}

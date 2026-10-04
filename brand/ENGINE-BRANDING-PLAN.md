# Engine branding plan (needs a Rust compile — NOT done)

Everything that needs no compile is already Copperline: PWA icons, manifest,
`game/engine/index.html` + `index.dedicated.html` (favicons, theme-color and the
HTML loading screen, now an inline `axenstax-mark-flat.svg` on Deep Frontier),
AppImage/Windows icons and the Android launcher icon. Regenerate rasters with
`node brand/render-rasters.mjs`. This file covers what remains inside the Rust engine.

## Where the engine shows brand today

| Surface | File / function | Today | Copperline replacement |
|---|---|---|---|
| Window icon | `game/engine/src/lib.rs` ~L1904, `Window::default_attributes().with_title("Axe'n'Stax")` | none set: Linux/Windows show a generic icon in the taskbar and alt-tab (AppImage desktop entry uses `tools/packaging/icons`, the running window does not) | `.with_window_icon(Some(Icon::from_rgba(..)))` from an `include_bytes!` 128px PNG (`brand/svg/axenstax-app-icon-rounded.svg`), decoded with the `image` crate (already a dep, see `cosmetics.rs`). `#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]` so it adds nothing to the web or APK builds. On Wayland also set the app id to `com.axenstax.engine` (winit `WindowAttributesExtWayland::with_name`) so the compositor matches the packaged `.desktop` icon. |
| Splash | `src/splash_ui.rs::draw_splash` (L41); `BG_COLOR` (10,14,22), title "AXE'N'STAX" at L80, "PROOF OF PLAY" tagline, loading bar | text + painted shapes only | Deep Frontier `#0D1B1E` (13,27,30) background; stone `#E6E0D1` title, copper `#D27B3E` tagline/bar; draw the mark texture above the title. |
| World-load screen | `src/loading_screen.rs`: `BG` (L164), `paint_voxel_hero` (L~190, four hard-coded voxel colours), title at L248 (243,230,207), tagline (212,160,68), bar (240,198,116) | painted 5-cube voxel diorama, mirrors the old HTML screen | Replace `paint_voxel_hero` with the mark texture so native matches the web page. Swap the constants to the brand tokens. |
| Main menu title | `src/menu.rs` L9-31 theme consts (`TITLE_COLOR` (212,160,68), `BG_DARK`, `CARD_*`, `ACTION_BLUE`), title label L2758 | gold-on-navy | `TITLE_COLOR` -> copper, `BG_DARK`/`PANEL_BG` -> Deep Frontier tints, `CARD_SELECTED_BORDER`/`ACTION_BLUE` -> sky blue `#3F7FBF` or lantern. Optionally the horizontal lockup (`axenstax-horizontal-dark.svg`) in place of the text label. |
| Showcase title | `src/showcase_ui.rs` L54 | text | same colour swap |
| egui theme | `src/egui_integration.rs` ~L28 (`Visuals::dark()`, widget fills, focus stroke (255,210,80), selection (80,120,200)) | generic blue-grey | widget fills from Deep Rock `#2B2B2B`/Deep Frontier, focus stroke lantern `#F4C16F`, selection forest green `#2E6B43`. Per-UI consts also in `hud_ui.rs` L16 and `craft_ui.rs` L13: do a separate pass, they are gameplay-critical contrast. |
| HTML loading screen | `game/engine/index.html`, `index.dedicated.html` | DONE in this change | - |

## How to ship the logo texture

Do not add the vector to egui (no SVG loader dep). Pre-rasterise in `brand/render-rasters.mjs`
(add one job: mark at 2x, about 336x246, transparent) to e.g. `game/engine/assets/brand/mark.png`,
`include_bytes!` it, decode with `image::load_from_memory`, upload once with `ctx.load_texture`
and cache it in `SplashState`/`LoadingState`. Share one loader in a small `brand.rs` so splash,
loading screen and menu use the same handle. These are egui textures: **separate from the block
texture array**, so the 256-layer array limit / WebGPU crash guard does not apply, and no atlas
slot is used.

## Bundle-size estimate (gate: brotli total < 5 MiB)

- Mark texture PNG: roughly 25-45 KB (PNG does not compress further under brotli) -> under 1% of the budget.
- Window icon 128px PNG: about 15 KB, native only, **0 bytes in the WASM bundle**.
- Colour constant swaps: 0 bytes.
Total worst case about 50 KB. Confirm with `tools/smoke/bundle-size.mjs` (via `./check.sh`) after the change.

## Verification needed (all require a build, so deferred)

1. `./check.sh` green (clippy `-D warnings`, `cargo test --lib`, trunk build, bundle gate).
2. Native run on Linux X11 and Wayland: taskbar/alt-tab icon is the Copperline tile, splash, load screen and menu render with brand colours; check legibility of dim text (`DIM_TEXT` (100,100,100) on the new bg) and the focus ring contrast.
3. WASM at `/game`: no regression in boot gate; loading screen swap from HTML to engine screen has no colour flash (HTML bg is already `#0D1B1E`).
4. Android: install the APK built with `tools/packaging/android/build-apk.sh` and look at the launcher (adaptive circle/squircle and the legacy PNG on API 24-25), then splash on device (`sensorLandscape`, DPI).
5. Existing UI tests that assert on colours/strings (grep `loading_screen`, `splash_ui`, `menu` tests) updated alongside; the word guards (no money/earn words) still run in `trials_lint`.
6. Screenshot comparison before/after, owner sign-off on the palette change: it touches every menu, so land it as its own commit.

## Not in scope here / noticed

- Tagline "PROOF OF PLAY" remains on the splash, the load screen and both HTML pages. It is current copy, so it is left alone; revisit if the Copperline direction drops it.
- `index.dedicated.html` ships no favicon (the page deliberately has no `/static` assets). An inline `data:` SVG favicon would work, but needs a check against the dedicated server's CSP first.
- The game site's `auth-ui.css` and `_debug-monitor.js` still use the old gold `#d4a044`; they live under `tools/sites/` and were out of scope.

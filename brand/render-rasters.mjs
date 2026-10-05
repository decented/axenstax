#!/usr/bin/env node
// Renders every raster derivative of the brand SVGs. One script, so all of them
// stay reproducible:  node brand/render-rasters.mjs
//
// Paths below are repo-relative. Source SVGs live in brand/svg/.
//
// Website / social (brand/png/, copied to the sites by sync-sites.sh):
//   favicon-32.png           <- axenstax-favicon.svg           32
//   apple-touch-icon-180.png <- axenstax-app-icon-rounded.svg  180, flattened on #0D1B1E
//   og-1200x630.png          <- axenstax-stacked-dark.svg      on #0D1B1E
// Game PWA (tools/sites/game/static/icons/, same filenames the manifest and
// game/engine/index.html already point at):
//   favicon-32.png, favicon.ico (16/32/48)     <- axenstax-favicon.svg
//   favicon-180.png                            <- rounded tile flattened on #0D1B1E
//   favicon-192/512.png  ("any")               <- axenstax-app-icon.svg (full square)
//   maskable-512.png                           <- detailed mark inside the 80% safe circle on #0D1B1E
// Desktop / AppImage (tools/packaging/icons/):
//   icon-512.png                               <- axenstax-app-icon-rounded.svg (transparent corners)
//   icon.ico (16,32 favicon; 48..256 rounded tile)
// Android (tools/packaging/android/res/mipmap-*/):
//   ic_launcher.png            48/72/96/144/192  <- rounded tile (legacy launchers, API 24-25)
//   ic_launcher_foreground.png 108/162/216/324/432 <- detailed mark, transparent, inside the
//                                                    central 66/108 safe circle (adaptive icon;
//                                                    background colour is res/values/colors.xml)
//
// Rust engine (game/engine/assets/brand/, include_bytes! by src/brand.rs):
//   mark.png          <- axenstax-mark-flat.svg, 336x246 transparent (splash + loading screens)
//   app-icon-128.png  <- axenstax-app-icon-rounded.svg 128, transparent corners (native window icon)
//
// Uses Playwright from tools/smoke/node_modules (no rsvg/inkscape needed).
import { createRequire } from 'node:module';
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(here, '..');
const require = createRequire(path.join(root, 'tools', 'smoke', 'package.json'));
const { chromium } = require('playwright');

const DEEP = '#0D1B1E';
const svgText = (name) => readFileSync(path.join(here, 'svg', name), 'utf8');
const svgUri = (name) =>
  'data:image/svg+xml;base64,' + Buffer.from(svgText(name)).toString('base64');
const img = (name, w, h = w) => `<img src="${svgUri(name)}" width="${w}" height="${h}" style="display:block">`;

// Playwright's bundled Chromium if it is installed, else the system Google Chrome.
const browser = await chromium.launch().catch(() => chromium.launch({ channel: 'chrome' }));
const MARK_W = 600, MARK_H = 440; // axenstax-mark.svg canvas

// Content bounding box of the detailed mark inside its 600x440 canvas.
async function markBox() {
  const page = await browser.newPage();
  await page.setContent(`<!doctype html><body>${svgText('axenstax-mark.svg')}</body>`);
  const b = await page.evaluate(() => {
    const r = document.querySelector('svg').getBBox();
    return { x: r.x, y: r.y, w: r.width, h: r.height };
  });
  await page.close();
  return b;
}
const MB = await markBox();

// The detailed mark centred on a size x size canvas, scaled so the mark's bounding
// box is inscribed in a circle of `circleFrac * size` diameter (Android adaptive
// safe zone = 66/108; PWA maskable safe zone = 80%).
function markInCircle(size, circleFrac) {
  const d = size * circleFrac;
  const s = d / Math.hypot(MB.w, MB.h);
  const left = size / 2 - (MB.x + MB.w / 2) * s;
  const top = size / 2 - (MB.y + MB.h / 2) * s;
  return `<div style="position:relative;width:${size}px;height:${size}px">
    <img src="${svgUri('axenstax-mark.svg')}" width="${MARK_W * s}" height="${MARK_H * s}"
         style="position:absolute;left:${left}px;top:${top}px"></div>`;
}

async function render(w, h, bg, html) {
  const page = await browser.newPage({ viewport: { width: w, height: h }, deviceScaleFactor: 1 });
  try {
    await page.setContent(
      `<!doctype html><html><body style="margin:0;background:${bg};overflow:hidden">${html}</body></html>`);
    await page.waitForFunction(() => [...document.images].every((i) => i.complete && i.naturalWidth > 0));
    return await page.screenshot({ type: 'png', omitBackground: bg === 'transparent',
                                   clip: { x: 0, y: 0, width: w, height: h } });
  } finally {
    await page.close();
  }
}

// ICONDIR + one ICONDIRENTRY per image, PNG bytes as payload (Vista+ / all browsers).
function packIco(images) {
  const n = images.length;
  const head = Buffer.alloc(6 + 16 * n);
  head.writeUInt16LE(0, 0); head.writeUInt16LE(1, 2); head.writeUInt16LE(n, 4);
  let offset = head.length;
  images.forEach(({ size, buf }, i) => {
    const e = 6 + 16 * i;
    head[e] = size >= 256 ? 0 : size; head[e + 1] = size >= 256 ? 0 : size;
    head.writeUInt16LE(1, e + 4); head.writeUInt16LE(32, e + 6);
    head.writeUInt32LE(buf.length, e + 8); head.writeUInt32LE(offset, e + 12);
    offset += buf.length;
  });
  return Buffer.concat([head, ...images.map((i) => i.buf)]);
}

function write(rel, buf) {
  const p = path.join(root, rel);
  mkdirSync(path.dirname(p), { recursive: true });
  writeFileSync(p, buf);
  console.log('wrote', rel, buf.length, 'bytes');
}

const favicon = (n) => render(n, n, 'transparent', img('axenstax-favicon.svg', n));
const rounded = (n, bg = 'transparent') => render(n, n, bg, img('axenstax-app-icon-rounded.svg', n));
const square = (n) => render(n, n, DEEP, img('axenstax-app-icon.svg', n));

try {
  // --- website / social ---
  write('brand/png/favicon-32.png', await favicon(32));
  // iOS paints transparent corners black, so flatten onto the tile's own colour.
  write('brand/png/apple-touch-icon-180.png', await rounded(180, DEEP));
  write('brand/png/og-1200x630.png', await render(1200, 630, DEEP,
    // stacked lockup is 740x634; 540 wide -> ~463 tall, centred.
    `<div style="width:1200px;height:630px;display:flex;align-items:center;justify-content:center">
       ${img('axenstax-stacked-dark.svg', 540, 463)}</div>`));

  // --- game PWA ---
  const pwa = 'tools/sites/game/static/icons/';
  const fav16 = await favicon(16), fav32 = await favicon(32), fav48 = await favicon(48);
  write(pwa + 'favicon-32.png', fav32);
  write(pwa + 'favicon.ico', packIco([{ size: 16, buf: fav16 }, { size: 32, buf: fav32 }, { size: 48, buf: fav48 }]));
  write(pwa + 'favicon-180.png', await rounded(180, DEEP));
  write(pwa + 'favicon-192.png', await square(192));
  write(pwa + 'favicon-512.png', await square(512));
  write(pwa + 'maskable-512.png', await render(512, 512, DEEP, markInCircle(512, 0.8)));

  // --- desktop / AppImage / Windows ---
  const pk = 'tools/packaging/icons/';
  write(pk + 'icon-512.png', await rounded(512));
  write(pk + 'icon.ico', packIco([
    { size: 16, buf: fav16 }, { size: 32, buf: fav32 },
    { size: 48, buf: await rounded(48) }, { size: 64, buf: await rounded(64) },
    { size: 128, buf: await rounded(128) }, { size: 256, buf: await rounded(256) },
  ]));

  // --- Rust engine (egui mark texture + native window icon) ---
  const eng = 'game/engine/assets/brand/';
  write(eng + 'mark.png', await render(336, 246, 'transparent', img('axenstax-mark-flat.svg', 336, 246)));
  write(eng + 'app-icon-128.png', await rounded(128));

  // --- Android ---
  const dens = { mdpi: 1, hdpi: 1.5, xhdpi: 2, xxhdpi: 3, xxxhdpi: 4 };
  for (const [d, k] of Object.entries(dens)) {
    const dir = `tools/packaging/android/res/mipmap-${d}/`;
    write(dir + 'ic_launcher.png', await rounded(48 * k));
    write(dir + 'ic_launcher_foreground.png',
      await render(108 * k, 108 * k, 'transparent', markInCircle(108 * k, 66 / 108)));
  }
} finally {
  await browser.close();
}

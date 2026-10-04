#!/usr/bin/env node
// Renders the raster derivatives of the brand SVGs into brand/png/.
//   favicon-32.png           <- svg/axenstax-favicon.svg           32x32
//   apple-touch-icon-180.png <- svg/axenstax-app-icon-rounded.svg  180x180
//   og-1200x630.png          <- svg/axenstax-stacked-dark.svg on #0D1B1E, 1200x630
// Uses Playwright from tools/smoke/node_modules (no rsvg/inkscape needed).
// Run:  node brand/render-rasters.mjs
import { createRequire } from 'node:module';
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(path.join(here, '..', 'tools', 'smoke', 'package.json'));
const { chromium } = require('playwright');

const svgUri = (name) =>
  'data:image/svg+xml;base64,' + readFileSync(path.join(here, 'svg', name)).toString('base64');
mkdirSync(path.join(here, 'png'), { recursive: true });

const jobs = [
  { out: 'favicon-32.png', w: 32, h: 32, bg: 'transparent',
    html: `<img src="${svgUri('axenstax-favicon.svg')}" width="32" height="32">` },
  // iOS paints transparent corners black, so flatten onto the tile's own colour.
  { out: 'apple-touch-icon-180.png', w: 180, h: 180, bg: '#0D1B1E',
    html: `<img src="${svgUri('axenstax-app-icon-rounded.svg')}" width="180" height="180">` },
  { out: 'og-1200x630.png', w: 1200, h: 630, bg: '#0D1B1E',
    // stacked lockup is 740x634; 540 wide -> ~463 tall, centred.
    html: `<div style="width:1200px;height:630px;display:flex;align-items:center;justify-content:center">
             <img src="${svgUri('axenstax-stacked-dark.svg')}" width="540" height="463"></div>` },
];

// Playwright's bundled Chromium if it is installed, else the system Google Chrome.
const browser = await chromium.launch().catch(() => chromium.launch({ channel: 'chrome' }));
try {
  for (const j of jobs) {
    const page = await browser.newPage({ viewport: { width: j.w, height: j.h }, deviceScaleFactor: 1 });
    await page.setContent(
      `<!doctype html><html><body style="margin:0;background:${j.bg};overflow:hidden">${j.html}</body></html>`);
    await page.waitForFunction(() => [...document.images].every((i) => i.complete && i.naturalWidth > 0));
    const buf = await page.screenshot({ type: 'png', omitBackground: j.bg === 'transparent',
                                        clip: { x: 0, y: 0, width: j.w, height: j.h } });
    writeFileSync(path.join(here, 'png', j.out), buf);
    console.log('wrote brand/png/' + j.out, buf.length, 'bytes');
    await page.close();
  }
} finally {
  await browser.close();
}

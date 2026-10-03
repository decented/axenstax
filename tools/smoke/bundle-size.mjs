#!/usr/bin/env node
// Measure the WASM+JS bundle size in the trunk dist dir.
// Reports raw, gzip, brotli. Exit 1 if brotli > 5 MB (spec gate).

import { readdirSync, readFileSync, statSync } from "node:fs";
import { gzipSync, brotliCompressSync, constants as zlibConstants } from "node:zlib";
import { resolve } from "node:path";

const DIST = resolve(new URL(".", import.meta.url).pathname, "..", "..", "game", "engine", "dist");
const BROTLI_GATE_BYTES = 5 * 1024 * 1024;

function human(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 / 1024).toFixed(2)} MiB`;
}

let totalRaw = 0;
let totalGzip = 0;
let totalBrotli = 0;
const rows = [];

for (const name of readdirSync(DIST).sort()) {
  const path = resolve(DIST, name);
  if (!statSync(path).isFile()) continue;
  if (!/\.(wasm|js|html|css)$/.test(name)) continue;

  const buf = readFileSync(path);
  const raw = buf.length;
  const gz = gzipSync(buf, { level: 9 }).length;
  const br = brotliCompressSync(buf, {
    params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 11 },
  }).length;

  totalRaw += raw;
  totalGzip += gz;
  totalBrotli += br;
  rows.push({ name, raw, gz, br });
}

console.log("File                                        raw        gzip       brotli");
console.log("-".repeat(78));
for (const r of rows) {
  console.log(
    `${r.name.padEnd(44)}${human(r.raw).padStart(10)} ${human(r.gz).padStart(10)} ${human(r.br).padStart(10)}`,
  );
}
console.log("-".repeat(78));
console.log(
  `${"TOTAL".padEnd(44)}${human(totalRaw).padStart(10)} ${human(totalGzip).padStart(10)} ${human(totalBrotli).padStart(10)}`,
);

if (totalBrotli > BROTLI_GATE_BYTES) {
  console.error(
    `\nFAIL: brotli total ${human(totalBrotli)} exceeds gate of ${human(BROTLI_GATE_BYTES)}`,
  );
  process.exit(1);
}
console.log(`\nOK: brotli total under ${human(BROTLI_GATE_BYTES)} gate`);

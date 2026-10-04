#!/usr/bin/env bash
# Copy the brand assets the websites need from brand/ into each in-scope site's
# static/brand/ folder. Run after changing anything under brand/svg, brand/png
# or brand/fonts (regenerate rasters first: node brand/render-rasters.mjs).
#
# (copper-line.svg and the UI icons are not synced: no page uses them yet.)
# Out of scope on purpose: tools/sites/claim (deliberately isolated),
# tools/sites/console. The game PWA icons (static/icons/), desktop and Android icons are
# written straight to their consumer paths by render-rasters.mjs, not copied here.
set -euo pipefail

BRAND="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SITES="$BRAND/../tools/sites"
SITE_LIST=(marketing project docs wiki learn game)

SVGS=(
  axenstax-mark-flat.svg
  axenstax-wordmark-dark.svg
  axenstax-stacked-dark.svg
  axenstax-favicon.svg
  axenstax-app-icon-rounded.svg
)
PNGS=(favicon-32.png apple-touch-icon-180.png og-1200x630.png)
FONTS=(inter-latin-wght-normal.woff2 noto-sans-display-latin-wdth-normal.woff2 OFL.txt)

for f in "${SVGS[@]}";  do [ -f "$BRAND/svg/$f" ]   || { echo "missing brand/svg/$f" >&2; exit 1; }; done
for f in "${PNGS[@]}";  do [ -f "$BRAND/png/$f" ]   || { echo "missing brand/png/$f (run node brand/render-rasters.mjs)" >&2; exit 1; }; done
for f in "${FONTS[@]}"; do [ -f "$BRAND/fonts/$f" ] || { echo "missing brand/fonts/$f" >&2; exit 1; }; done

for site in "${SITE_LIST[@]}"; do
  dest="$SITES/$site/static/brand"
  rm -rf "$dest"
  mkdir -p "$dest/fonts"
  for f in "${SVGS[@]}";  do cp "$BRAND/svg/$f"   "$dest/$f"; done
  for f in "${PNGS[@]}";  do cp "$BRAND/png/$f"   "$dest/$f"; done
  for f in "${FONTS[@]}"; do cp "$BRAND/fonts/$f" "$dest/fonts/$f"; done
  echo "synced $site -> static/brand/"
done

# The shared stylesheet must stay byte-identical across the six sites.
want="$(md5sum < "$SITES/game/static/style.css")"
for site in "${SITE_LIST[@]}"; do
  got="$(md5sum < "$SITES/$site/static/style.css")"
  if [ "$got" != "$want" ]; then
    echo "WARNING: $site/static/style.css differs from game/static/style.css (cp it over)" >&2
  fi
done

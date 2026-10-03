#!/usr/bin/env python3
"""Generate the bundled sample texture pack ("vivid") for the game site.

A texture pack only ships the textures it overrides (Spec 03 §11.4), so this is
a handful of bright 16x16 PNGs plus an animated water strip + `.anim` sidecar.
It demonstrates BOTH:
  - web pack-swapping  — pick "vivid" in the in-game Graphics settings (PWA),
  - native animation   — copy this dir into `texturepacks/` and water animates.

Run:  python3 tools/texturepack-sample/generate.py
Writes into tools/sites/game/static/packs/vivid/ + the packs index.json.
Outputs are committed; rerun only when changing the sample.
"""
import json
import os

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
PACKS = os.path.normpath(os.path.join(HERE, "..", "sites", "game", "static", "packs"))
PACK = os.path.join(PACKS, "vivid")

# key -> solid RGBA colour (bright + distinct so the swap is obvious)
SOLID = {
    "blocks/stone": (96, 112, 140, 255),
    "blocks/dirt": (124, 84, 48, 255),
    "blocks/sand": (236, 214, 120, 255),
    "blocks/grass_top": (92, 184, 72, 255),
    "blocks/grass_side": (112, 160, 84, 255),
}
# Animated water: a 16x(16*frames) vertical strip cycling two blues.
WATER_FRAMES = [(40, 92, 200, 255), (64, 124, 232, 255)]


def _checker(base):
    """A subtle 2-tone 16x16 so it reads as a texture, not a flat fill."""
    img = Image.new("RGBA", (16, 16), base)
    dark = tuple(max(0, c - 18) for c in base[:3]) + (255,)
    px = img.load()
    for y in range(16):
        for x in range(16):
            if (x // 4 + y // 4) % 2 == 0:
                px[x, y] = dark
    return img


def main():
    files = []
    for key, colour in SOLID.items():
        path = os.path.join(PACK, key + ".png")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        _checker(colour).save(path)
        files.append(key)

    # Animated water strip (vertical) + .anim sidecar.
    strip = Image.new("RGBA", (16, 16 * len(WATER_FRAMES)))
    for i, colour in enumerate(WATER_FRAMES):
        strip.paste(_checker(colour), (0, i * 16))
    os.makedirs(os.path.join(PACK, "blocks"), exist_ok=True)
    strip.save(os.path.join(PACK, "blocks/water.png"))
    with open(os.path.join(PACK, "blocks/water.png.anim"), "w") as fh:
        json.dump({"frame_time": 6}, fh, indent=2)
    files.append("blocks/water")

    with open(os.path.join(PACK, "pack.json"), "w") as fh:
        json.dump(
            {
                "name": "vivid",
                "description": "Bundled sample pack — bright solids + animated water.",
                "version": "1.0.0",
                "texture_resolution": 16,
                "authors": ["AxeNStax"],
                "license": "CC0-1.0",
            },
            fh,
            indent=2,
        )

    # Aggregated web discovery index — one fetch lists every pack + its files.
    index = [{"name": "vivid", "resolution": 16, "files": files}]
    with open(os.path.join(PACKS, "index.json"), "w") as fh:
        json.dump(index, fh, indent=2)

    print(f"wrote {len(files)} textures to {PACK}")
    print(f"index: {os.path.join(PACKS, 'index.json')}")


if __name__ == "__main__":
    main()

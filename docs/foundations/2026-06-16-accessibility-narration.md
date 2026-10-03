# Accessibility narration — hear the HUD (Web Speech TTS)

**Status**: ✅ BUILT 2026-06-16 (worktree `worktree-community-features`, goal `2026-06-16-community-features-solo-buildout`). Phase 1 feature **#24** — a differentiator + ethics win: a screen-reader-style narration so a player who can't read the HUD can still navigate.
**Date**: 2026-06-16
**Backlog**: `docs/research/2026-06-15-native-bake-in-feature-backlog.md` §24.

---

## TL;DR

An opt-in **Web Speech (TTS)** narration that speaks the **hotbar selection** as it changes ("Slot 3: Stone"). The decision of *what* to narrate is a pure, tested formatter + a throttle; the actual `speak()` uses the browser's `speechSynthesis` (web-only — a no-op on native, where OS TTS is a follow-up). Off by default; toggled in Settings.

## Design (concrete, not cards)

- **`narration.rs`** — pure core (3 tests): `hotbar_phrase`/`health_phrase` (the phrasings), `Narrator` (throttle: only emits a phrase when it changes, so the same slot isn't re-spoken every frame). `speak(text)` is `#[cfg(wasm)]` → `window().speech_synthesis()` (cancels the in-flight utterance first, so navigation stays responsive); native is a no-op.
- **Setting** — `GraphicsSettings.narration_enabled` (default **off**, personal pref, preset-excluded) + a Settings checkbox.
- **Trigger** — each tick, when enabled, build the hotbar phrase for player 0's selected slot + item (via the block/item registry) → `Narrator::next` → `speak`. Speaks on change, throttled.
- **web-sys** — added the `SpeechSynthesis` + `SpeechSynthesisUtterance` features (engine-only Cargo change; no JS-site edit).

## Solo boundary → playtest gate

Solo: the phrasings + throttle are unit-tested; the wiring + the wasm `speak()` compile (trunk-built). **Playtest** (owner, needs the PWA + audio): the actual TTS voice/rate/clarity, whether hotbar-only is enough or health/UI/menu narration is wanted, and the 3D sound-cue orientation below.

## Deferred (named — the rest of the backlog item)

- **Broader narration**: health on damage, menu/UI focus, block-looked-at, container contents.
- **3D sound-cue orientation** (the backlog's spatial-audio half) — directional audio cues for nearby entities/hazards; its own audio-engine work.
- **Voice / rate / pitch** settings; a native OS-TTS backend (so it's not web-only).

## Spec maintenance

Spec 05 (Gameplay) accessibility note references the narration toggle; the web-only constraint + deferred spatial cues are captured here.

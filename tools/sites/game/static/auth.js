// Axe'n'Stax — PWA boot (web local-sandbox tier).
//
// Web login was RETIRED 2026-06-27: the web build is a login-free local sandbox
// (a taster). Identity, cloud saves (Stash) and multiplayer live in the native
// download. This module's whole job is now: (1) gate on WebGPU, then (2) boot the
// WASM engine as an anonymous guest. No Signet, no /auth round-trip, no cookie,
// no signer. The engine reads no WASM_PUBKEY, so every world is namespaced
// "local" (see save::wasm_storage_key).
// Spec: docs/superpowers/specs/2026-06-27-web-local-sandbox-design.md
//
// Globals the WASM side registers (during #[wasm_bindgen(start)]):
//   window.__axenstax_start()        — spawns the winit event loop (called below)
// Globals this module publishes (consumed by the engine):
//   window.axenstax_exit_to_lobby()  — engine lobby "Exit" → the marketing home

(function () {
    'use strict';

    const loadingEl = document.getElementById('loading');
    const statusEl = document.getElementById('status');
    const webgpuGate = document.getElementById('webgpu-gate');
    function setStatus(msg) { if (statusEl) statusEl.textContent = msg; }
    function showLoading() { if (loadingEl) loadingEl.classList.remove('hidden'); }

    function escapeHtml(s) {
        return String(s)
            .replace(/&/g, '&amp;')
            .replace(/</g, '&lt;')
            .replace(/>/g, '&gt;')
            .replace(/"/g, '&quot;');
    }
    function reportError(msg) {
        console.error(msg);
        const el = document.getElementById('errors');
        if (el) { el.style.display = 'block'; el.innerHTML += escapeHtml(msg) + '<br>'; }
    }

    // Engine lobby "Exit" → leave the game for the marketing home (axenstax.com).
    // Since the restructure, play.axenstax.com/ IS the game, so navigating to '/'
    // would just reload back into the lobby. The real "home" is now the marketing
    // site, injected by app.py as <meta name="marketing-url">.
    // (`wasm_auth::exit_to_lobby` calls this.)
    window.axenstax_exit_to_lobby = function () {
        const m = document.querySelector('meta[name="marketing-url"]');
        location.href = (m && m.content) ? m.content : 'https://axenstax.com';
    };

    // Boot the WASM engine as a guest. Wait for the engine to register its start
    // hook (in #[wasm_bindgen(start)]), then start the event loop. No pubkey is
    // set — the web tier is anonymous by default.
    async function bootGuest() {
        setStatus('Loading engine — playing as guest…');
        showLoading();
        const deadline = Date.now() + 15000;
        while (typeof window.__axenstax_start !== 'function') {
            if (Date.now() > deadline) {
                reportError('WASM engine failed to register its boot hook within 15 s');
                return;
            }
            await new Promise(r => setTimeout(r, 20));
        }
        window.__axenstax_start();
    }

    // WebGPU gate — on success boot the engine; on failure show the FRIENDLY
    // browser-specific #webgpu-gate panel (ported from the retired entrance) so the
    // visitor gets clear "update Chrome / fix drivers / Re-test" guidance instead of
    // a raw red error bar. The probe + copy live in webgpu-check.js (window.AxeWebGpu),
    // loaded just before this file; this module owns the boot decision.
    async function runGate() {
        // Defensive fallback if webgpu-check.js failed to load — fail safe with the
        // old raw probe + red-bar message rather than silently doing nothing.
        if (!window.AxeWebGpu) {
            if (!navigator.gpu) {
                reportError('WebGPU is not available on this browser/device. The game requires Chromium with WebGPU enabled.');
                setStatus('WebGPU not available');
                return;
            }
            try {
                const adapter = await navigator.gpu.requestAdapter();
                if (!adapter) {
                    reportError('No WebGPU adapter found. Your GPU may not be supported.');
                    setStatus('No GPU adapter');
                    return;
                }
                // Defence in depth: mirrors the maxTextureArrayLayers floor in
                // webgpu-check.js (this branch only runs if that script failed to
                // load). Without it a low-layer-budget adapter would sail through
                // this fallback and hit the renderer's block-texture-atlas panic.
                // NOTE: this 512 is one of FOUR hardcoded copies of the floor —
                // keep in sync with webgpu-check.js's REQUIRED_TEXTURE_ARRAY_LAYERS,
                // webgpu-probe.js, and index.dedicated.html. The Rust drift-guard
                // (texture_gen::texture_count_stays_under_webgpu_check_floor) names
                // all four and fails if texture_count() grows past 512.
                const layers = adapter.limits && adapter.limits.maxTextureArrayLayers;
                if (typeof layers === 'number' && layers < 512) {
                    reportError('Your graphics device only exposes ' + layers + ' texture array layers (need 512+). Try updating your GPU drivers.');
                    setStatus('GPU texture-layer limit too low');
                    return;
                }
            } catch (e) {
                reportError('GPU adapter error: ' + (e && e.message ? e.message : e));
                return;
            }
            if (webgpuGate) webgpuGate.hidden = true;
            showLoading();
            bootGuest();
            return;
        }

        const result = await window.AxeWebGpu.probe();
        if (result.ok) {
            if (webgpuGate) webgpuGate.hidden = true;
            showLoading();
            bootGuest();
            return;
        }
        // Blocked: hide the loader, reveal the friendly gate with per-browser copy.
        // Deliberately NOT reportError here — the raw red #errors bar is the unfriendly
        // path we're replacing. (reportError stays for the genuine 15 s boot timeout.)
        if (loadingEl) loadingEl.classList.add('hidden');
        if (webgpuGate) webgpuGate.hidden = false;
        window.AxeWebGpu.showBlockFor(document.getElementById('webgpu-block'), result.hasGpuApi, result.reason);
    }

    // Re-test button: re-probe in place so a user can flip a flag / update their
    // browser and validate without hunting for a refresh.
    const retestBtn = document.getElementById('webgpu-retest');
    if (retestBtn) {
        retestBtn.addEventListener('click', function () {
            retestBtn.disabled = true;
            const original = retestBtn.textContent;
            retestBtn.textContent = 'Testing…';
            Promise.resolve(runGate()).finally(function () {
                retestBtn.disabled = false;
                retestBtn.textContent = original;
            });
        });
    }

    runGate();
})();

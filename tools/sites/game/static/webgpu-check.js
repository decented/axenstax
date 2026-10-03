// Pre-flight WebGPU probe for the lobby.
//
// Calls navigator.gpu.requestAdapter() once on lobby load. If it succeeds,
// nothing is shown. If it fails, the #webgpu-block panel is revealed with
// browser-specific guidance and the "Enter the game" CTA (signed-in state)
// is hard-disabled so a click can't dump the user into a worse engine error.
//
// The Re-test button reruns the probe in place — the user can toggle a flag
// or restart their browser and validate without hunting for a refresh.

(function () {
    'use strict';

    // Minimum `adapter.limits.maxTextureArrayLayers` we require before letting
    // the engine boot. The block-texture atlas allocates one array layer per
    // `texture_gen::texture_count()` (game/engine/src/texture_gen.rs) — currently
    // 459. Some adapters (SwiftShader software fallback, some integrated/mobile
    // GPUs) only expose the WebGPU spec's default floor of 256, which is below
    // that, and `Renderer::new` hard-panics trying to allocate the atlas on such
    // a device (see `assert_block_texture_layers_fit` in renderer.rs).
    //
    // 512 gives headroom above 459 for near-term block-registry growth while
    // staying at/below what real desktop GPUs report (Intel/NVIDIA/AMD commonly
    // expose 2048), so it only blocks genuinely-incapable devices.
    //
    // DRIFT GUARD: game/engine/src/texture_gen.rs has a `#[cfg(test)]` test
    // (`texture_count_stays_under_webgpu_check_floor`) asserting
    // `texture_count() <= 512`. If a future wave grows the atlas past 512, that
    // test fails loudly — bump BOTH this constant and the Rust test's floor
    // together.
    const REQUIRED_TEXTURE_ARRAY_LAYERS = 512;

    async function detectBrowser() {
        const ua = navigator.userAgent;
        // Mobile is informational, not a block: the engine has touch controls
        // (virtual joystick, look zone, hotbar, jump/break/place) and modern
        // mobile Chrome has WebGPU. We tweak per-browser remediation copy when
        // isMobile is true, but we never gate a mobile user out on browser alone.
        const isMobile = /Android|iPhone|iPad|Mobile/i.test(ua);
        // Brave is also a Chrome UA, so it must be checked first.
        if (navigator.brave && typeof navigator.brave.isBrave === 'function') {
            try {
                const isBrave = await navigator.brave.isBrave();
                if (isBrave) return { id: 'brave', name: 'Brave', isMobile: isMobile };
            } catch (_e) { /* fall through to UA sniffing */ }
        }
        if (/Firefox\//.test(ua)) return { id: 'firefox', name: 'Firefox', isMobile: isMobile };
        if (/Edg\//.test(ua))     return { id: 'edge',    name: 'Edge',    isMobile: isMobile };
        if (/OPR\//.test(ua))     return { id: 'opera',   name: 'Opera',   isMobile: isMobile };
        if (/Chrome\//.test(ua))  return { id: 'chrome',  name: 'Chrome',  isMobile: isMobile };
        if (/Safari\//.test(ua))  return { id: 'safari',  name: 'Safari',  isMobile: isMobile };
        return { id: 'unknown', name: 'this browser', isMobile: isMobile };
    }

    // A flag address shown as <code> plus a one-click Copy button. A real
    // hyperlink can't work — browsers block web pages from navigating to
    // internal `brave://`/`chrome://` pages — so Copy is the substitute: tap it,
    // then paste into a new tab. (Click handler wired in renderFix; inline
    // onclick would break the strict CSP.)
    function flagButton(url) {
        return '<code>' + url + '</code>'
            + ' <button type="button" class="webgpu-copy" data-copy="' + url + '"'
            + ' style="margin-left:.4rem;padding:.1rem .55rem;font-size:.8rem;cursor:pointer;'
            + 'border:1px solid #2c6a36;border-radius:6px;background:#14241a;color:#6dd57a;">Copy</button>';
    }

    function fixForBrowser(browser, hasGpuApi, reason) {
        // Insufficient texture-array-layer budget is a device/driver limitation,
        // not really a per-browser one — a software (SwiftShader) fallback
        // renderer reports the same low limit regardless of which Chromium
        // browser is asking. Give it one clear, distinct message rather than
        // routing it through the "couldn't reach your GPU" per-browser copy.
        if (reason === 'texture-layers') {
            return {
                headline: 'Your graphics device doesn\'t support enough texture layers to run Axe\'n\'Stax.',
                steps: [
                    'This usually means your browser fell back to a software renderer (e.g. SwiftShader) instead of your real graphics card.',
                    'Update your GPU drivers (NVIDIA / AMD / Intel) to the latest version and restart your browser fully.',
                    'On Linux: install Vulkan tooling (<code>sudo apt install mesa-vulkan-drivers vulkan-tools</code>) and run <code>vulkaninfo --summary</code> — if that errors, fix Vulkan first.',
                    'Come back here and hit <strong>Re-test</strong>.',
                ],
                fallback: 'If it still fails, this hardware may not be able to run Axe\'n\'Stax in a browser — try a different device with a dedicated or more recent GPU.',
            };
        }
        const mobile = !!browser.isMobile;
        switch (browser.id) {
            case 'brave': {
                const isLinux = /Linux/i.test(navigator.userAgent) && !mobile;
                if (mobile) {
                    return {
                        headline: 'Brave needs WebGPU switched on to run the game.',
                        steps: [
                            'Open a new tab and paste this address: ' + flagButton('brave://flags/#enable-unsafe-webgpu') + ' — set it to <strong>Enabled</strong>.',
                            'Force-quit Brave and reopen.',
                            'Come back here and hit <strong>Re-test</strong>.',
                        ],
                        fallback: 'If it still fails, try Chrome on this device — it has WebGPU on by default on supported phones.',
                    };
                }
                if (isLinux) {
                    return {
                        headline: 'Brave needs one flag flipped to reach your graphics card.',
                        steps: [
                            'Paste this into a new Brave tab: ' + flagButton('brave://flags/#enable-vulkan') + ' — set it to <strong>Enabled</strong>. (This is what lets Brave use your GPU on Linux, so the game runs smoothly rather than crawling on the CPU.)',
                            'Restart Brave fully — close <em>every</em> window, then reopen.',
                            'Come back here and hit <strong>Re-test</strong>.',
                        ],
                        fallback: 'Still no luck? Also enable ' + flagButton('brave://flags/#enable-unsafe-webgpu') + ' and restart again — that forces WebGPU on even when Brave isn\'t sure about your GPU (it may run slower). If it\'s still stuck, your GPU drivers may need updating.',
                    };
                }
                return {
                    headline: 'Brave gates WebGPU behind a flag — turn it on and you\'re in.',
                    steps: [
                        'Paste this into a new Brave tab: ' + flagButton('brave://flags/#enable-unsafe-webgpu') + ' — set it to <strong>Enabled</strong>.',
                        'Restart Brave fully — close every window, reopen.',
                        'Come back here and hit <strong>Re-test</strong>.',
                    ],
                    fallback: 'If it still fails, your GPU drivers may be the holdup — Chrome and Edge are slightly more permissive on the same hardware.',
                };
            }
            case 'firefox':
                return {
                    headline: 'Firefox doesn\'t run Axe\'n\'Stax yet.',
                    steps: mobile
                        ? [
                            'Install <strong>Chrome</strong> on this device and open <code>' + window.location.origin + '</code> there.',
                        ]
                        : [
                            'Install <strong>Chrome</strong>, <strong>Edge</strong>, or <strong>Brave</strong>.',
                            'Open <code>' + window.location.origin + '</code> there and sign in again.',
                        ],
                    fallback: 'Firefox WebGPU support is shipping in stages — we\'ll add it when it\'s broadly available.',
                };
            case 'safari':
                // On iOS every browser is WebKit under the hood — the alpha is Chromium-only
                // per ADR-003 regardless of WebGPU availability in Safari 26+.
                return {
                    headline: mobile
                        ? 'iOS isn\'t supported for the alpha.'
                        : 'Safari doesn\'t run Axe\'n\'Stax yet.',
                    steps: mobile
                        ? [
                            'The alpha is Chromium-only, and every iOS browser is WebKit underneath.',
                            'Open <code>' + window.location.origin + '</code> on an Android phone or a desktop running Chrome / Edge / Brave.',
                        ]
                        : [
                            'Install <strong>Chrome</strong>, <strong>Edge</strong>, or <strong>Brave</strong>.',
                            'Open <code>' + window.location.origin + '</code> there and sign in again.',
                        ],
                };
            case 'opera':
                return {
                    headline: 'Opera doesn\'t expose WebGPU here.',
                    steps: [
                        'Try <strong>Chrome</strong>, <strong>Edge</strong>, or <strong>Brave</strong> on the same device.',
                        'If you stay on Opera, check <code>opera://flags</code> for a WebGPU toggle and restart.',
                    ],
                };
            case 'chrome':
                return {
                    headline: hasGpuApi
                        ? 'Chrome has WebGPU but couldn\'t reach your GPU.'
                        : 'Your Chrome is too old for WebGPU.',
                    steps: hasGpuApi
                        ? (mobile
                            ? [
                                'Update Chrome from the Play Store and reopen this page.',
                                'Some older Android GPUs aren\'t on the WebGPU allow-list yet — if updating doesn\'t fix it, try a different device.',
                            ]
                            : [
                                'Update Chrome to the latest stable, restart it fully, and re-test.',
                                'On Linux: install Vulkan tooling (<code>sudo apt install mesa-vulkan-drivers vulkan-tools</code>) and run <code>vulkaninfo --summary</code> — if that errors, fix Vulkan first.',
                                'Check that your GPU drivers are current (NVIDIA / AMD / Intel).',
                            ])
                        : [
                            'Update Chrome to <strong>121 or newer</strong>.',
                            'Restart it and re-test.',
                        ],
                };
            case 'edge':
                return {
                    headline: hasGpuApi
                        ? 'Edge has WebGPU but couldn\'t reach your GPU.'
                        : 'Your Edge is too old for WebGPU.',
                    steps: hasGpuApi
                        ? (mobile
                            ? [
                                'Update Edge from the Play Store and reopen this page.',
                                'If that doesn\'t fix it, try Chrome on the same device.',
                            ]
                            : [
                                'Update graphics drivers, restart Edge fully, and re-test.',
                                'On Linux: install Vulkan tooling (<code>sudo apt install mesa-vulkan-drivers vulkan-tools</code>) and run <code>vulkaninfo --summary</code>.',
                            ])
                        : [
                            'Update Edge to <strong>121 or newer</strong>.',
                            'Restart it and re-test.',
                        ],
                };
            default:
                return {
                    headline: 'This browser doesn\'t expose WebGPU.',
                    steps: mobile
                        ? [
                            'Install a recent build of <strong>Chrome</strong> on this device and reopen <code>' + window.location.origin + '</code>.',
                        ]
                        : [
                            'Install a recent build of <strong>Chrome</strong>, <strong>Edge</strong>, or <strong>Brave</strong> on a desktop.',
                            'Open <code>' + window.location.origin + '</code> there and sign in.',
                        ],
                };
        }
    }

    async function probe() {
        if (!navigator.gpu) return { ok: false, hasGpuApi: false };
        let adapter;
        try {
            adapter = await navigator.gpu.requestAdapter();
        } catch (_e) {
            return { ok: false, hasGpuApi: true };
        }
        if (!adapter) return { ok: false, hasGpuApi: true };
        // The adapter exists, but may still not be able to hold the block-texture
        // atlas (see REQUIRED_TEXTURE_ARRAY_LAYERS comment above) — a passing
        // adapter here does NOT guarantee `Renderer::new` can succeed.
        const layers = adapter.limits && adapter.limits.maxTextureArrayLayers;
        if (typeof layers === 'number' && layers < REQUIRED_TEXTURE_ARRAY_LAYERS) {
            return { ok: false, hasGpuApi: true, reason: 'texture-layers', layers: layers };
        }
        return { ok: true, hasGpuApi: true };
    }

    function renderFix(panel, browser, hasGpuApi, reason) {
        const lede = panel.querySelector('#webgpu-block-lede');
        const fix = panel.querySelector('#webgpu-block-fix');
        const info = fixForBrowser(browser, hasGpuApi, reason);
        if (lede) lede.textContent = info.headline;
        if (!fix) return;
        let html = '<ol class="webgpu-steps">';
        for (const step of info.steps) {
            html += '<li>' + step + '</li>';
        }
        html += '</ol>';
        if (info.fallback) {
            html += '<p class="webgpu-fallback">' + info.fallback + '</p>';
        }
        fix.innerHTML = html;
        // Wire the Copy buttons (handlers here, not inline — CSP forbids inline).
        fix.querySelectorAll('.webgpu-copy').forEach(function (btn) {
            btn.addEventListener('click', function () {
                const text = btn.getAttribute('data-copy') || '';
                if (navigator.clipboard && navigator.clipboard.writeText) {
                    navigator.clipboard.writeText(text).then(function () {
                        const orig = btn.textContent;
                        btn.textContent = 'Copied ✓';
                        setTimeout(function () { btn.textContent = orig; }, 1500);
                    }).catch(function () { btn.textContent = 'Copy failed — select it manually'; });
                } else {
                    btn.textContent = 'Copy failed — select it manually';
                }
            });
        });
    }

    function disableEnterCta() {
        const enter = document.querySelector('.cta-enter');
        if (!enter) return;
        if (enter.dataset.webgpuOriginalHref === undefined) {
            enter.dataset.webgpuOriginalHref = enter.getAttribute('href') || '/game';
            enter.dataset.webgpuOriginalText = enter.textContent;
        }
        enter.classList.add('cta-blocked');
        enter.removeAttribute('href');
        enter.setAttribute('role', 'button');
        enter.setAttribute('aria-disabled', 'true');
        enter.textContent = 'Browser can\'t run the engine yet';
        if (!enter.dataset.webgpuClickBound) {
            enter.addEventListener('click', function (e) {
                if (enter.classList.contains('cta-blocked')) {
                    e.preventDefault();
                    const panel = document.getElementById('webgpu-block');
                    if (panel) panel.scrollIntoView({ behavior: 'smooth', block: 'center' });
                }
            });
            enter.dataset.webgpuClickBound = '1';
        }
    }

    function enableEnterCta() {
        const enter = document.querySelector('.cta-enter');
        if (!enter || !enter.classList.contains('cta-blocked')) return;
        enter.classList.remove('cta-blocked');
        enter.setAttribute('href', enter.dataset.webgpuOriginalHref || '/game');
        enter.removeAttribute('role');
        enter.removeAttribute('aria-disabled');
        if (enter.dataset.webgpuOriginalText) {
            enter.textContent = enter.dataset.webgpuOriginalText;
        }
    }

    async function runCheck() {
        const panel = document.getElementById('webgpu-block');
        if (!panel) return;
        const result = await probe();
        if (result.ok) {
            panel.hidden = true;
            enableEnterCta();
            return;
        }
        const browser = await detectBrowser();
        renderFix(panel, browser, result.hasGpuApi, result.reason);
        panel.hidden = false;
        disableEnterCta();
    }

    function init() {
        const retestBtn = document.getElementById('webgpu-retest');
        if (retestBtn) {
            retestBtn.addEventListener('click', function () {
                retestBtn.disabled = true;
                const original = retestBtn.textContent;
                retestBtn.textContent = 'Testing…';
                runCheck().finally(function () {
                    retestBtn.disabled = false;
                    retestBtn.textContent = original;
                });
            });
        }
        runCheck();
    }

    // Page-agnostic API for the game page (game/engine/index.html), where auth.js
    // owns the probe + boot + Re-test wiring. `showBlockFor` renders the friendly
    // browser-specific copy into a #webgpu-block panel without touching any CTA.
    async function showBlockFor(panel, hasGpuApi, reason) {
        if (!panel) return;  // defensive: nothing to render into (caller passed null)
        const b = await detectBrowser();
        renderFix(panel, b, hasGpuApi, reason);
    }
    window.AxeWebGpu = { probe: probe, showBlockFor: showBlockFor };

    // Entrance-only auto-init: only self-drive (probe + CTA-gate) on a page that
    // has the entrance's `.cta-enter` button. The game page has no such CTA — there
    // auth.js drives the gate via window.AxeWebGpu, so this file stays inert.
    if (document.querySelector('.cta-enter')) {
        if (document.readyState === 'loading') {
            document.addEventListener('DOMContentLoaded', init);
        } else {
            init();
        }
    }
})();

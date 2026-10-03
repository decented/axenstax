// Standalone WebGPU probe for the marketing /will-it-run page.
//
// Independent of the game-site lobby's webgpu-check.js — the marketing site
// is an entirely separate FastAPI app with its own static dir, and this page
// has no Enter-button to disable. Logic is similar but the DOM contract is
// minimal: one #status node and one #fix node that this script populates.

(function () {
    'use strict';

    async function detectBrowser() {
        const ua = navigator.userAgent;
        const isMobile = /Android|iPhone|iPad|Mobile/i.test(ua);
        if (navigator.brave && typeof navigator.brave.isBrave === 'function') {
            try {
                if (await navigator.brave.isBrave()) return { id: 'brave', name: 'Brave', isMobile: isMobile };
            } catch (_e) { /* fall through */ }
        }
        if (/Firefox\//.test(ua)) return { id: 'firefox', name: 'Firefox', isMobile: isMobile };
        if (/Edg\//.test(ua))     return { id: 'edge',    name: 'Edge',    isMobile: isMobile };
        if (/OPR\//.test(ua))     return { id: 'opera',   name: 'Opera',   isMobile: isMobile };
        if (/Chrome\//.test(ua))  return { id: 'chrome',  name: 'Chrome',  isMobile: isMobile };
        if (/Safari\//.test(ua))  return { id: 'safari',  name: 'Safari',  isMobile: isMobile };
        return { id: 'unknown', name: 'this browser', isMobile: isMobile };
    }

    // Same floor as tools/sites/game/static/webgpu-check.js
    // (REQUIRED_TEXTURE_ARRAY_LAYERS) — see that file's comment and the Rust
    // drift-guard test (texture_gen::texture_count_stays_under_webgpu_check_floor)
    // for why 512. Keep in sync if the block atlas grows.
    const REQUIRED_TEXTURE_ARRAY_LAYERS = 512;

    function fixForBrowser(browser, hasGpuApi, reason) {
        if (reason === 'texture-layers') {
            return {
                headline: 'Your graphics device doesn\'t support enough texture layers to run Axe\'n\'Stax.',
                steps: [
                    'This usually means your browser fell back to a software renderer (e.g. SwiftShader) instead of your real graphics card.',
                    'Update your GPU drivers (NVIDIA / AMD / Intel) to the latest version and restart your browser fully, then re-test.',
                ],
            };
        }
        const mobile = !!browser.isMobile;
        switch (browser.id) {
            case 'brave':
                return {
                    headline: 'Brave gates WebGPU behind a flag.',
                    steps: mobile
                        ? [
                            'Open <code>brave://flags</code>, search <strong>WebGPU</strong>, set to Enabled.',
                            'Force-quit Brave and reopen.',
                            'Reload this page and re-test.',
                        ]
                        : [
                            'Open <code>brave://flags/#enable-webgpu</code>.',
                            'Set it to <strong>Enabled</strong>.',
                            'Restart Brave fully — close every window, reopen.',
                            'Come back and re-test.',
                        ],
                };
            case 'firefox':
                return {
                    headline: 'Firefox isn\'t supported yet.',
                    steps: [
                        'Use <strong>Chrome</strong>, <strong>Edge</strong>, or <strong>Brave</strong> instead.',
                    ],
                };
            case 'safari':
                return {
                    headline: mobile ? 'iOS isn\'t supported yet.' : 'Safari isn\'t supported yet.',
                    steps: [
                        'Use <strong>Chrome</strong>, <strong>Edge</strong>, or <strong>Brave</strong>' + (mobile ? ' on Android, or a desktop' : '') + '.',
                    ],
                };
            case 'opera':
                return {
                    headline: 'Opera doesn\'t expose WebGPU here.',
                    steps: [
                        'Try Chrome, Edge, or Brave on the same device.',
                    ],
                };
            case 'chrome':
                return {
                    headline: hasGpuApi ? 'Chrome has WebGPU but couldn\'t reach your GPU.' : 'Your Chrome is too old.',
                    steps: hasGpuApi
                        ? (mobile
                            ? ['Update Chrome from the Play Store.', 'Some older Android GPUs aren\'t on the WebGPU allow-list yet.']
                            : ['Update Chrome to the latest stable.', 'On Linux: install Vulkan tooling (<code>sudo apt install mesa-vulkan-drivers vulkan-tools</code>) and check <code>vulkaninfo --summary</code>.', 'Update GPU drivers.'])
                        : ['Update Chrome to <strong>121 or newer</strong>.'],
                };
            case 'edge':
                return {
                    headline: hasGpuApi ? 'Edge has WebGPU but couldn\'t reach your GPU.' : 'Your Edge is too old.',
                    steps: hasGpuApi
                        ? (mobile ? ['Update Edge from the Play Store.'] : ['Update GPU drivers and restart Edge.', 'On Linux: install Vulkan tooling.'])
                        : ['Update Edge to <strong>121 or newer</strong>.'],
                };
            default:
                return {
                    headline: 'This browser doesn\'t expose WebGPU.',
                    steps: ['Use a recent <strong>Chrome</strong>, <strong>Edge</strong>, or <strong>Brave</strong>' + (mobile ? '' : ' on a desktop') + '.'],
                };
        }
    }

    async function probe() {
        if (!navigator.gpu) return { ok: false, hasGpuApi: false, info: null };
        try {
            const adapter = await navigator.gpu.requestAdapter();
            if (!adapter) return { ok: false, hasGpuApi: true, info: null };
            let info = null;
            try {
                const a = adapter;
                info = a.info || (typeof a.requestAdapterInfo === 'function' ? await a.requestAdapterInfo() : null);
            } catch (_e) { /* info is best-effort */ }
            // An adapter existing isn't enough to actually run the game — it must
            // also be able to hold the block-texture atlas, or the engine's
            // Renderer::new hard-panics allocating it.
            const layers = adapter.limits && adapter.limits.maxTextureArrayLayers;
            if (typeof layers === 'number' && layers < REQUIRED_TEXTURE_ARRAY_LAYERS) {
                return { ok: false, hasGpuApi: true, info: info, reason: 'texture-layers' };
            }
            return { ok: true, hasGpuApi: true, info: info };
        } catch (_e) {
            return { ok: false, hasGpuApi: true, info: null };
        }
    }

    function render(result, browser) {
        const status = document.getElementById('status');
        const fix = document.getElementById('fix');
        const cta = document.getElementById('go-cta');
        if (!status || !fix) return;

        if (result.ok) {
            status.className = 'verdict verdict-ok';
            const adapter = result.info && (result.info.vendor || result.info.architecture || result.info.description);
            const adapterLine = adapter ? '<p class="adapter">Adapter: ' + escapeHtml(JSON.stringify({ vendor: result.info.vendor, architecture: result.info.architecture, device: result.info.device, description: result.info.description })) + '</p>' : '';
            status.innerHTML = '<h2>Looks good — your browser can run Axe\'n\'Stax.</h2><p>WebGPU is available and the engine should boot.</p>' + adapterLine;
            fix.innerHTML = '';
            if (cta) { cta.style.display = 'inline-block'; cta.textContent = 'Try the alpha →'; }
            return;
        }

        status.className = 'verdict verdict-blocked';
        const info = fixForBrowser(browser, result.hasGpuApi, result.reason);
        status.innerHTML = '<h2>Not yet on this browser.</h2><p>' + info.headline + '</p>';
        let html = '<h3>Try this</h3><ol>';
        for (const step of info.steps) html += '<li>' + step + '</li>';
        html += '</ol>';
        fix.innerHTML = html;
        if (cta) cta.style.display = 'none';
    }

    function escapeHtml(s) {
        return String(s)
            .replace(/&/g, '&amp;')
            .replace(/</g, '&lt;')
            .replace(/>/g, '&gt;')
            .replace(/"/g, '&quot;')
            .replace(/'/g, '&#39;');
    }

    async function runCheck() {
        const result = await probe();
        const browser = await detectBrowser();
        render(result, browser);
    }

    function init() {
        const retest = document.getElementById('retest');
        if (retest) {
            retest.addEventListener('click', function () {
                retest.disabled = true;
                const original = retest.textContent;
                retest.textContent = 'Testing…';
                runCheck().finally(function () {
                    retest.disabled = false;
                    retest.textContent = original;
                });
            });
        }
        runCheck();
    }

    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', init);
    } else {
        init();
    }
})();

// Fullscreen toggle — floating corner button.
// Fullscreen is requested on document.documentElement so the canvas and this
// button both remain visible. Entering fullscreen requires a user gesture,
// which is why the click handler runs entirely in JS (no WASM round-trip).
(function () {
    'use strict';

    function install() {
        if (document.getElementById('fs-toggle')) return;
        const btn = document.createElement('button');
        btn.id = 'fs-toggle';
        btn.type = 'button';
        btn.title = 'Toggle fullscreen (Esc to exit)';
        btn.setAttribute('aria-label', 'Toggle fullscreen');
        btn.textContent = '\u26F6'; // ⛶ expand glyph
        btn.addEventListener('click', () => {
            if (document.fullscreenElement) {
                document.exitFullscreen().catch(e => console.warn('exitFullscreen:', e));
            } else {
                const target = document.documentElement;
                (target.requestFullscreen
                    ? target.requestFullscreen()
                    : Promise.reject(new Error('requestFullscreen unsupported'))
                ).catch(e => console.warn('requestFullscreen:', e));
            }
        });
        document.body.appendChild(btn);
        document.addEventListener('fullscreenchange', () => {
            btn.textContent = document.fullscreenElement ? '\u2715' : '\u26F6';
            btn.title = document.fullscreenElement
                ? 'Exit fullscreen (Esc)'
                : 'Enter fullscreen';
        });
    }

    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', install);
    } else {
        install();
    }
})();

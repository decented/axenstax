// Branded "enter the game" loading screen.
//
// Rotates Tip / What's-New cards while the WASM bundle downloads. Reads the
// SAME card list the engine bakes in (game/engine/assets/loading_tips.json,
// trunk-copied to /loading_tips.json) so the two loading surfaces never drift.
// Pure-JS, CSP-friendly (external 'self' script + same-origin fetch). The
// engine hides #loading via auth.js `hideLoading()` once it's ready; we just
// animate until then. Everything is guarded so a fetch failure leaves the hero
// + progress bar showing (just no card).
(function () {
  'use strict';

  var CARD_HOLD_MS = 6000; // matches loading_screen::CARD_HOLD_SECS in the engine
  var SWAP_MS = 300;       // crossfade duration (must match #ls-card transition)

  // Fisher–Yates permutation of 0..n, re-seated so the first element never
  // equals `prevLast` (no card repeats across a reshuffle boundary).
  function shuffleNoRepeat(n, prevLast) {
    var v = [];
    for (var i = 0; i < n; i++) v.push(i);
    for (var i = n - 1; i > 0; i--) {
      var j = Math.floor(Math.random() * (i + 1));
      var t = v[i]; v[i] = v[j]; v[j] = t;
    }
    if (n > 1 && prevLast != null && v[0] === prevLast) {
      var k = 1 + Math.floor(Math.random() * (n - 1));
      var t2 = v[0]; v[0] = v[k]; v[k] = t2;
    }
    return v;
  }

  function start() {
    var card = document.getElementById('ls-card');
    if (!card) return;
    var kEl = card.querySelector('.k');
    var tEl = card.querySelector('.t');
    var bEl = card.querySelector('.b');
    if (!kEl || !tEl || !bEl) return;

    fetch('/loading_tips.json', { cache: 'no-cache' })
      .then(function (r) { return r.ok ? r.json() : Promise.reject(r.status); })
      .then(function (cards) {
        if (!Array.isArray(cards) || cards.length === 0) return;
        var order = shuffleNoRepeat(cards.length, null);
        var pos = 0;

        function render() {
          var c = cards[order[pos]] || {};
          var isNew = c.kind === 'new';
          kEl.className = 'k ' + (isNew ? 'new' : 'tip');
          kEl.textContent = isNew ? '✨ NEW — NEEDS TESTING' : '💡 TIP';
          tEl.textContent = c.title || '';
          bEl.textContent = c.body || '';
        }

        function advance() {
          // Skip work once the engine has hidden the overlay.
          var loading = document.getElementById('loading');
          if (loading && loading.classList.contains('hidden')) return;
          card.classList.add('swap'); // fade out
          setTimeout(function () {
            var last = order[pos];
            pos++;
            if (pos >= order.length) { order = shuffleNoRepeat(cards.length, last); pos = 0; }
            render();
            card.classList.remove('swap'); // fade in
          }, SWAP_MS);
        }

        render();
        card.hidden = false;
        setInterval(advance, CARD_HOLD_MS);
      })
      .catch(function () { /* no card — hero + bar still show */ });
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start);
  } else {
    start();
  }
})();

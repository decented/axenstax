// Print-button handler for /print/test-sheet/*. Strict-CSP-friendly — no
// inline handlers, no external deps. Just wires window.print() to the
// "Print this sheet" button.
(function () {
    'use strict';
    var btn = document.getElementById('print-btn');
    if (btn) {
        btn.addEventListener('click', function () { window.print(); });
    }
})();

// Homepage "Get the desktop app" door — adapt to the visitor's OS.
//
// Default markup is the Linux state (Download for Linux → the /download page).
// On Windows/Mac we don't have an installer yet, and on phones the app is
// desktop-only — both swap to a "coming soon / play in your browser" state so
// nobody dead-ends. Linux/unknown desktop keeps the download.
(function () {
  var door = document.getElementById('door-native');
  if (!door) return;
  var cta = document.getElementById('native-cta');
  var fb = document.getElementById('native-fallback');
  var gameUrl = door.getAttribute('data-game') || '/';

  var ua = (navigator.userAgent || '').toLowerCase();
  var plat = (
    (navigator.userAgentData && navigator.userAgentData.platform) ||
    navigator.platform || ''
  ).toLowerCase();

  var isAndroid = ua.indexOf('android') !== -1;
  // iPadOS ≥13 reports as Mac but is a touch device → treat as iOS.
  var isIOS = /iphone|ipad|ipod/.test(ua) ||
    (plat.indexOf('mac') !== -1 && (navigator.maxTouchPoints || 0) > 1);
  var isMobile = isAndroid || isIOS || /mobile/.test(ua);
  var isWin = !isMobile && (plat.indexOf('win') !== -1 || ua.indexOf('windows') !== -1);
  var isMac = !isMobile && !isWin && (plat.indexOf('mac') !== -1 || ua.indexOf('mac os') !== -1);

  function disableCta(label) {
    cta.textContent = label;
    cta.classList.add('cta-secondary', 'is-soon');
    cta.removeAttribute('href');
    cta.setAttribute('role', 'text');
    cta.setAttribute('aria-disabled', 'true');
  }
  function showFallback(html) {
    if (!fb) return;
    fb.innerHTML = html;
    fb.hidden = false;
  }

  if (isWin || isMac) {
    disableCta((isWin ? 'Windows' : 'Mac') + ' app — coming soon');
    showFallback('Play in your browser meanwhile — <a href="' + gameUrl + '">open the game &rarr;</a>');
  } else if (isMobile) {
    disableCta('The app is desktop-only');
    showFallback('On a phone? <a href="' + gameUrl + '">Play in your browser &rarr;</a>');
  }
  // Linux / unknown desktop → leave the default "Download for Linux" → /download.
})();

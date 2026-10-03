// Operator Console — dashboard actions. Each POST hits an operator-gated
// /api/* endpoint and reloads on success (simple + always-consistent).
(function () {
  "use strict";

  function form(obj) {
    var fd = new FormData();
    Object.keys(obj).forEach(function (k) { fd.append(k, obj[k]); });
    return fd;
  }

  async function doAndReload(url, body, btn) {
    if (btn) btn.disabled = true;
    try {
      var res = await fetch(url, {
        method: "POST",
        headers: { "X-Requested-With": "fetch" },
        body: body,
      });
      if (!res.ok) {
        var d = "";
        try { d = (await res.json()).detail || ""; } catch (e) { /* ignore */ }
        alert("Failed (" + res.status + ")" + (d ? " — " + d : ""));
        if (btn) btn.disabled = false;
        return;
      }
      location.reload();
    } catch (e) {
      alert("Error: " + (e && e.message ? e.message : e));
      if (btn) btn.disabled = false;
    }
  }

  var rs = document.getElementById("require-signin");
  if (rs) {
    rs.addEventListener("change", function () {
      doAndReload("api/require-signin", form({ on: rs.checked ? "true" : "false" }));
    });
  }

  document.querySelectorAll("form[data-add]").forEach(function (f) {
    f.addEventListener("submit", function (e) {
      e.preventDefault();
      var npub = f.querySelector("input[name=npub]").value.trim();
      if (!npub) return;
      doAndReload("api/list/" + f.getAttribute("data-add") + "/add", form({ npub: npub }), f.querySelector("button"));
    });
  });

  document.querySelectorAll("button[data-remove]").forEach(function (b) {
    b.addEventListener("click", function () {
      doAndReload("api/list/" + b.getAttribute("data-remove") + "/remove", form({ npub: b.getAttribute("data-npub") }), b);
    });
  });

  // Team / roles — add or remove an owner/admin/moderator.
  document.querySelectorAll("form[data-team-add]").forEach(function (f) {
    f.addEventListener("submit", function (e) {
      e.preventDefault();
      var role = f.getAttribute("data-team-add");
      var npub = f.querySelector("input[name=npub]").value.trim();
      if (!npub) return;
      doAndReload("api/team/" + role + "/add", form({ npub: npub }), f.querySelector("button"));
    });
  });

  document.querySelectorAll("button[data-team-remove]").forEach(function (b) {
    b.addEventListener("click", function () {
      var role = b.getAttribute("data-team-remove");
      if (!confirm("Remove this " + role + "?")) return;
      doAndReload("api/team/" + role + "/remove", form({ npub: b.getAttribute("data-npub") }), b);
    });
  });

  // Studio / Gallery — upload (multipart) + delete exhibit images.
  var su = document.getElementById("studio-upload");
  if (su) {
    su.addEventListener("submit", function (e) {
      e.preventDefault();
      var fi = su.querySelector("input[name=file]");
      if (!fi.files || !fi.files.length) { alert("Choose an image first."); return; }
      var fd = new FormData();
      fd.append("file", fi.files[0]);
      doAndReload("api/studio/upload", fd, su.querySelector("button"));
    });
  }

  document.querySelectorAll("button[data-studio-delete]").forEach(function (b) {
    b.addEventListener("click", function () {
      var name = b.getAttribute("data-studio-delete");
      if (!confirm("Delete " + name + "?")) return;
      doAndReload("api/studio/delete", form({ name: name }), b);
    });
  });

  // Copy a ready-to-paste /exhibit place command to the clipboard (the artist
  // pastes it into the in-game chat, or sends it to a helper). Falls back to a
  // hidden textarea + execCommand on older / non-secure-context browsers.
  document.querySelectorAll("button[data-copy]").forEach(function (b) {
    b.addEventListener("click", function () {
      var text = b.getAttribute("data-copy");
      var done = function () {
        var old = b.textContent;
        b.textContent = "Copied ✓";
        setTimeout(function () { b.textContent = old; }, 1500);
      };
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(done, function () { fallback(text, done); });
      } else {
        fallback(text, done);
      }
    });
  });
  function fallback(text, done) {
    var ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    try { document.execCommand("copy"); done(); } catch (e) { /* no-op */ }
    document.body.removeChild(ta);
  }

  // World / Showcase — arm or disarm the kiosk live.
  var sc = document.getElementById("showcase-form");
  if (sc) {
    sc.addEventListener("submit", function (e) {
      e.preventDefault();
      var en = sc.querySelector("input[name=enabled]");
      doAndReload("api/showcase", form({
        enabled: en && en.checked ? "true" : "false",
        exit_action: sc.exit_action.value,
        auto_loop_secs: sc.auto_loop_secs.value,
      }), sc.querySelector("button"));
    });
  }

  // Gallery — "Build mode ⇄ Open to visitors" switch (arms/disarms the web kiosk).
  var go = document.getElementById("gallery-open");
  if (go) go.addEventListener("click", function () {
    doAndReload("api/gallery/visitors", form({ open: "true" }), go);
  });
  var gb = document.getElementById("gallery-build");
  if (gb) gb.addEventListener("click", function () {
    if (!confirm("Switch back to build mode? Visitors won't be able to look until you open it again.")) return;
    doAndReload("api/gallery/visitors", form({ open: "false" }), gb);
  });

  // Share link — the visitor URL is this same origin's root (the game), since the
  // console lives at /admin on it. The browser knows the real host, so use it.
  var shareLink = document.getElementById("share-link");
  if (shareLink) shareLink.textContent = window.location.origin + "/";
  var copyShare = document.getElementById("copy-share");
  if (copyShare) copyShare.addEventListener("click", function () {
    var text = window.location.origin + "/";
    var done = function () {
      var old = copyShare.textContent;
      copyShare.textContent = "Copied ✓";
      setTimeout(function () { copyShare.textContent = old; }, 1500);
    };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(done, function () { fallback(text, done); });
    } else { fallback(text, done); }
  });

  var kf = document.getElementById("kick-form");
  if (kf) {
    kf.addEventListener("submit", function (e) {
      e.preventDefault();
      var npub = kf.querySelector("input[name=npub]").value.trim();
      if (!npub) return;
      if (!confirm("Kick " + npub + "?")) return;
      doAndReload("api/kick", form({ npub: npub }), kf.querySelector("button"));
    });
  }

  var sf = document.getElementById("settings-form");
  if (sf) {
    sf.addEventListener("submit", function (e) {
      e.preventDefault();
      var ann = sf.querySelector("input[name=announce]");
      doAndReload("api/settings", form({
        server_name: sf.server_name.value,
        about: sf.about.value,
        region: sf.region.value,
        max_players: sf.max_players.value,
        announce: ann && ann.checked ? "true" : "false",
      }), sf.querySelector("button"));
    });
  }

  var pf = document.getElementById("privacy-form");
  if (pf) {
    pf.addEventListener("submit", function (e) {
      e.preventDefault();
      doAndReload("api/settings", form({
        privacy_level: pf.privacy_level.value,
        privacy_retention_days: pf.privacy_retention_days.value,
      }), pf.querySelector("button"));
    });
  }

  document.querySelectorAll("button[data-forget]").forEach(function (b) {
    b.addEventListener("click", function () {
      doAndReload("api/forget", form({ npub: b.getAttribute("data-npub") }), b);
    });
  });

  var pg = document.getElementById("purge");
  if (pg) {
    pg.addEventListener("click", function () {
      if (!confirm("Erase ALL session history? This can't be undone.")) return;
      doAndReload("api/purge-history", form({}), pg);
    });
  }

  var lo = document.getElementById("logout");
  if (lo) {
    lo.addEventListener("click", async function () {
      try {
        await fetch("auth/logout", { method: "POST", headers: { "X-Requested-With": "fetch" } });
      } catch (e) { /* ignore */ }
      window.location.href = document.baseURI;
    });
  }
})();

// Setup wizard — a tiny multi-step flow. No per-step server round-trips: it
// collects the few choices client-side and POSTs them once at "Create".
(function () {
  "use strict";

  var defaults = {};
  try { defaults = JSON.parse(document.getElementById("wiz-defaults").textContent); }
  catch (e) { defaults = {}; }

  // server_type → engine game mode (mirrors wizard.py SERVER_TYPES). Gallery runs as
  // a creative world (operator builds); the kiosk makes visitors read-only.
  var MODE = { gallery: "creative", creative: "creative", survival: "survival", adventure: "adventure" };
  var TYPE_LABEL = { gallery: "Gallery", creative: "Creative world", survival: "Survival world", adventure: "Adventure" };
  var ACCESS_LABEL = { open: "Anyone", signin: "Signed-in players", friends: "Just my friends" };

  // The steps that show a progress dot (the terminal "done" step doesn't).
  var STEPS = ["welcome", "type", "name", "access", "finish", "review"];

  var state = {
    server_type: defaults.server_type || "survival",
    server_name: defaults.server_name || "Axe'n'Stax Server",
    about: defaults.about || "",
    access: defaults.access || "open",
    announce: !!defaults.announce,
    keep_history: !!defaults.keep_history,
    friends: (defaults.friends || []).join("\n"),
  };

  function $(sel, root) { return (root || document).querySelector(sel); }
  function $all(sel, root) { return Array.prototype.slice.call((root || document).querySelectorAll(sel)); }
  function stepEl(name) { return $('.wiz-step[data-step="' + name + '"]'); }

  // ── progress dots ──
  var prog = $("#wiz-progress");
  STEPS.forEach(function () { var i = document.createElement("i"); prog.appendChild(i); });
  function renderProgress(current) {
    var idx = STEPS.indexOf(current);
    $all("i", prog).forEach(function (d, n) {
      d.className = n === idx ? "on" : (n < idx ? "done" : "");
    });
    prog.style.visibility = idx === -1 ? "hidden" : "visible";
  }

  var currentStep = "welcome";
  function show(name) {
    $all(".wiz-step").forEach(function (s) { s.hidden = s.getAttribute("data-step") !== name; });
    currentStep = name;
    renderProgress(name);
    if (name === "review") renderReview();
    window.scrollTo(0, 0);
  }

  // order helpers for Next/Back
  var FLOW = ["welcome", "type", "name", "access", "finish", "review"];
  function go(delta) {
    var i = FLOW.indexOf(currentStep);
    if (i === -1) return;
    var n = Math.max(0, Math.min(FLOW.length - 1, i + delta));
    show(FLOW[n]);
  }

  // ── prefill from defaults ──
  $("#f-name").value = state.server_name;
  $("#f-about").value = state.about;
  $("#f-announce").checked = state.announce;
  $("#f-history").checked = state.keep_history;
  $("#f-friends").value = state.friends;
  markChoice("#type-choices", state.server_type);
  markChoice("#access-choices", state.access);
  syncFriendsField();

  function markChoice(group, value) {
    $all(".choice", $(group)).forEach(function (b) {
      b.classList.toggle("sel", b.getAttribute("data-value") === value);
    });
  }
  function syncFriendsField() {
    $("#friends-field").hidden = state.access !== "friends";
  }

  // ── choice cards: select + auto-advance (one tap) ──
  $all(".choice", $("#type-choices")).forEach(function (b) {
    b.addEventListener("click", function () {
      state.server_type = b.getAttribute("data-value");
      markChoice("#type-choices", state.server_type);
      setTimeout(function () { go(1); }, 140);
    });
  });
  $all(".choice", $("#access-choices")).forEach(function (b) {
    b.addEventListener("click", function () {
      state.access = b.getAttribute("data-value");
      markChoice("#access-choices", state.access);
      syncFriendsField();
      // "friends" needs the npub box, so don't auto-skip past it.
      if (state.access !== "friends") setTimeout(function () { go(1); }, 140);
    });
  });

  // ── Next/Back ──
  $all("[data-next]").forEach(function (b) { b.addEventListener("click", function () { captureFields(); go(1); }); });
  $all("[data-back]").forEach(function (b) { b.addEventListener("click", function () { captureFields(); go(-1); }); });

  function captureFields() {
    state.server_name = $("#f-name").value.trim() || "Axe'n'Stax Server";
    state.about = $("#f-about").value.trim();
    state.announce = $("#f-announce").checked;
    state.keep_history = $("#f-history").checked;
    state.friends = $("#f-friends").value;
  }

  // ── review ──
  function renderReview() {
    captureFields();
    var dl = $("#review");
    var rows = [
      ["Kind of place", TYPE_LABEL[state.server_type] || state.server_type],
      ["Name", state.server_name],
    ];
    if (state.about) rows.push(["Description", state.about]);
    rows.push(["Who can join", ACCESS_LABEL[state.access] || state.access]);
    rows.push(["In server lists", state.announce ? "Yes" : "No"]);
    rows.push(["Visit history", state.keep_history ? "Kept for a week" : "Not kept"]);
    dl.innerHTML = "";
    rows.forEach(function (r) {
      var dt = document.createElement("dt"); dt.textContent = r[0];
      var dd = document.createElement("dd"); dd.textContent = r[1];
      dl.appendChild(dt); dl.appendChild(dd);
    });
    // Warn only when an existing world would change game mode (re-run).
    var newMode = MODE[state.server_type];
    var priorMode = defaults.game_mode;
    var needFresh = !!defaults.engine_booted && !!defaults.world_exists &&
      !!priorMode && priorMode !== newMode;
    $("#fresh-warn").hidden = !needFresh;
  }

  // ── POST helpers ──
  async function postJSON(url, body) {
    var res = await fetch(url, {
      method: "POST",
      headers: { "X-Requested-With": "fetch", "Content-Type": "application/json" },
      body: JSON.stringify(body || {}),
    });
    if (!res.ok) {
      var d = "";
      try { d = (await res.json()).detail || ""; } catch (e) { /* ignore */ }
      throw new Error("(" + res.status + ")" + (d ? " " + d : ""));
    }
    return res.json();
  }

  function dashboard() { return new URL("dashboard", document.baseURI).href; }

  // ── create ──
  $("#create").addEventListener("click", async function () {
    captureFields();
    var btn = this; btn.disabled = true;
    var payload = {
      server_type: state.server_type,
      server_name: state.server_name,
      about: state.about,
      access: state.access,
      announce: state.announce,
      keep_history: state.keep_history,
      friends: state.access === "friends" ? state.friends : "",
    };
    try {
      var r = await postJSON("api/setup/apply", payload);
      show("done");
      var msg = $("#done-msg"), status = $("#done-status"), title = $("#done-title");
      if (r.gallery) {
        title.textContent = "Your gallery is ready!";
        msg.textContent = "It starts in build mode — go in and place your art. When it's "
          + "ready, hit “Open to visitors” in the console and share the link.";
        status.textContent = "Starting your gallery… taking you to the console…";
      } else if (r.server_booting) {
        msg.textContent = r.fresh_world
          ? "Starting a fresh " + (TYPE_LABEL[r.server_type] || "") + "…"
          : "Your server is starting up…";
        status.textContent = "Taking you to the console…";
      } else {
        msg.textContent = "Your server is updated.";
        status.textContent = "Taking you to the console…";
      }
      setTimeout(function () { window.location.href = dashboard(); }, r.gallery ? 5000 : 3500);
    } catch (e) {
      btn.disabled = false;
      alert("Couldn't save setup " + (e && e.message ? e.message : e));
    }
  });

  // ── skip (small text, discouraged) ──
  $("#skip").addEventListener("click", async function (e) {
    e.preventDefault();
    try { await postJSON("api/setup/skip", {}); } catch (err) { /* ignore */ }
    window.location.href = dashboard();
  });

  show("welcome");
})();

# AxeNStax lobby ("world list") inventory, for redesign

Engine `game/engine/src/` (all paths below relative to that unless absolute).
Read at commit `eeec94b2` (v0.2.28). Read-only audit; nothing was run (no screenshots rendered).

Abbreviations: **native** = desktop + Android build (`cfg(not(wasm32))`); **web** = wasm32 build (the "local sandbox taster": no sign-in, no multiplayer, no Stash write).
`menu.rs` = `game/engine/src/menu.rs`. "L" numbers are menu.rs line numbers unless a file is named.

## 0. File sizes (lines)

| File | Lines | Role |
|---|---|---|
| menu.rs | 7203 | Lobby draw (`draw_main_menu` L2160-3383), world card (L3385-3600), all lobby dialogs, Stash column (L1791), pause menu (L4774, not lobby), Settings panel (L5865), sign-in dialog (L7011), `MenuAction` (L166), `MenuState` (L278) |
| game_loop.rs | 24381 | Consumes `MenuAction` (`match action` at L9342-10022); draws relays window + Settings over the lobby (L9166-9240); `reset_workshop_space` L3607 |
| lib.rs | 4244 | `--shot-lobby` fixture: `sample_menu_state` L3453, `run_shot_lobby` L3490 |
| friends_ui.rs | 873 | Friends column (native), "Your address", Add-a-friend copy, Host-online gating |
| trials_gallery_ui.rs | 1015 | Trials tile (L938) + full-screen Trials gallery (L766) |
| trials_gallery.rs | 824 | Gallery state / unlock fixtures |
| signet_contacts/ui.rs | 263 | "Signet contacts" row in Friends column + unreadable strip |
| relays_ui.rs | 432 | "Your relays" manager (Settings section + standalone window) |
| lan_ui.rs | 366 | "Games on this network" list + room-code copy in Join dialog |
| keeps.rs | 375 | My Keeps data + copy |
| native_signin.rs | 538 | Sign-in state machine behind the QR dialog |
| update_check.rs / self_update.rs | 604 / 718 | Version badge / in-place AppImage updater |
| launch_links.rs | 211 | `axenstax:` link held and offered to the lobby |
| handover_game.rs | (not counted) | "World handover" window, drawn over the lobby (L188) |
| brand.rs | 159 | wordmark / mark |
| menu_phone_panels_tests.rs / menu_keeps_tests.rs | 415 / 435 | phone-size layout tests (Settings, pause menu, Keeps) |

## 1. Layout and breakpoints (all in `draw_main_menu`)

Egui, points (not pixels). Panels are added in this order: right `stash_column`, right `friends_column`, top `signet_unreadable` strip, then `CentralPanel`. First-added right panel is outermost, so on a very wide window the order left-to-right is: world list | Friends & servers | The Stash.

| Width (content_rect) | What changes | file:line |
|---|---|---|
| < 600 | Wordmark 180 px wide instead of 220 | L2891 |
| < 900 | `wide=false`: Trials tile becomes full-width, 64 px tall (else max 420 px, 78 px tall). This is the ONLY use of `wide`. | L2795, L3035; trials_gallery_ui.rs:938 |
| >= 1080 | Right "The Stash" column, fixed 264 px (native AND web) | L2801-2822 |
| >= 1360 (`FRIENDS_COLUMN_MIN_WIDTH`) | Right "Friends & servers" column, fixed 248 px. Native only. | L2832-2849; signet_contacts/ui.rs:48 |
| < 1360 and Signet contacts unreadable | A top strip "Your Signet contacts can't be read." + Disconnect | L2855-2877 |
| any | World cards: columns = floor((avail_w+10)/330) clamped 1..3 (min card 320). Roughly: 1 col under ~690 total, 2 cols ~690 to ~1020, 3 cols only when avail >= 980 (about 1020-1079 with no side panels, ~1310+ with Stash only, ~1560+ with both). Estimates; exact panel chrome not measured. | L3288-3292 |
| height < 620 or Android | `dialog_is_compact`: dialogs become a height-capped, scrolling card with always-visible scrollbar; Settings window scrolls with a pinned Back | L4012-4018, L4020-4112; menu_phone_panels_tests.rs |

Fixture sizes (`--shot-lobby`): desktop 1280x800, tablet 834x1112, phone 390x844 (lib.rs:3497). Mapping: desktop = stash column yes, friends column NO (1280 < 1360); tablet 834 = no side columns, 2-column cards, narrow Trials tile; phone 390 = 1 column, 180 px wordmark. **No breakpoint exists for phone vs tablet beyond 600 (wordmark) and 900 (Trials tile).** There is no touch-specific lobby layout; touch only changes text fields (OS keyboard prompt on web: `touch_input::OS_KEYBOARD_PROMPT = cfg!(wasm32)`, touch_input.rs:67).

On phone and tablet widths (< 1080) the Stash column and the Friends column are absent entirely: no Browse an npub, no Official games card, no "Your address", no Add a friend, no Signet contacts, no My Servers. The only side-column content reachable on a phone is My Keeps, via the Join Game dialog. The code comment at L1805 notes the official games remain reachable in-world via `/scenario` (or the Trials gallery).

Scroll behaviour: the whole CentralPanel is NOT one scroll area. The header, Trials tile and the action row are fixed above; only the world card grid scrolls (`ScrollArea::vertical` L3281). A phone with the wrapped action row (up to 8 buttons on native) can therefore eat most of the screen height before any card shows.

## 2. Inventory table

"Shows when" shorthand: N = native only, W = web only, ALL = both. Android is native; Android specifics are called out.

### 2.1 Header block (top of CentralPanel)

| # | Label | What it does (MenuAction / effect) | Where | Shows when | file:line |
|---|---|---|---|---|---|
| H1 | Wordmark image | Branding only | Top-left of header | ALL | L2891 |
| H2 | `AxeNStax v0.2.28` version text (+ badge, see H3-H6) | Display only | Under wordmark | N only (web is "always current") | draw_version_line L2016-2096, called L2893 |
| H3 | `AxeNStax vX` plain, no badge | Update state `Unknown` (check in flight / offline / failed / opted out via `AXENSTAX_NO_UPDATE_CHECK`) | Under wordmark | N | L2041-2047; update_check.rs:48 |
| H4 | Pill `★ LATEST` | State `Current`: running newest published build | beside version | N | L2058-2065 |
| H5 | Pill `DEV BUILD` | State `Ahead`: build newer than anything published | beside version | N | L2067-2076 |
| H6 | Pill `UPDATE AVAILABLE` + link `(vX available)` | State `Available`. Link opens `update_check::DOWNLOAD_PAGE` in browser (no MenuAction) | beside version | N, newer version published | L2077-2095 |
| H7 | `Update now` (small button) | `self_update::start` downloads + verifies new AppImage in place. No MenuAction. | after H6 link | N, Available AND `$APPIMAGE` env set (running as a Linux AppImage) AND manifest has appimage url+sha256. Never on Android / web / deb / dev builds. | L2109-2128 |
| H8 | `Downloading vX... NN%` / `Checking the download...` / `Update failed: <reason>` | Progress text only. After `Update failed` the code draws just the text, no retry button this session (the doc comment promising a retry does not match the Failed arm, L2155) | after H6 | N, AppImage, update in flight | L2129-2158 |
| H9 | `Updated to vX —` + `Restart` (small button) | `MenuAction::RestartForUpdate`: normal exit path then relaunch | after H6 | N, AppImage, install finished | L2141-2152; game_loop.rs:10013 |
| H10 | `Your Worlds` subtitle | Label | Under version, left | ALL | L2902 |
| H11 | `Quit` | `MenuAction::Quit` -> `GameState::exit_app` (handover grace first) | Header right (rightmost) | N (incl. Android) | L2927 |
| H12 | `Exit` (tooltip "Leave the game, back to the home page") | `wasm_auth::exit_to_lobby()`: leaves the wasm game to the surrounding web page (a separate HTML "home page" outside this repo's egui code) | Header right | W only | L2917 |
| H13 | `Sign in` (tooltip "Sign in with Signet - scan a QR with your phone") | Resets `native_signin`, sets `MenuDialog::SignIn` -> QR dialog (section 2.6) | Header right, left of Quit | N, signed out | L2942 |
| H14 | `Sign out` (tooltip "Forget this device's Signet sign-in") | `native_signin::sign_out()`; no confirm | same slot as H13 | N, signed in | L2934 |
| H15 | `Settings` (tooltip "Graphics, your relays and more") | `show_settings = true`; caller draws the Settings window and the lobby paints background only | Header right, leftmost of the three | ALL (web panel is graphics-only) | L2960 |
| H16 | `👤 Playing as guest - Sign in to save to the cloud & join servers` | Label | Line under header | N, signed out | L2989 |
| H17 | `👤 npub1... (short)` (tooltip "Signed in with Signet - cloud save & multiplayer unlocked") | Label | Line under header | N, signed in | L2972-2984 |
| H18 | Stash sync status line (11.5 px green) | `state.stash_sync_msg`. Effectively dead: the only writer is `poll_cloud_worlds` (web) and the web build has no sync button; `stash_syncing` is `#[allow(dead_code)]` | Under identity line | ALL, practically never | L3007; field L384-L392 |
| H19 | Notice banner: any `state.notice` text + `OK` | Dismisses. See section 5 for every producer. For the Signet-unreadable host refusal it also grows a `Disconnect Signet` button. | Top-centre floating Area, 12 px from top, foreground | ALL when a notice is set | L2212-2236 |
| H20 | Loading veil `Loading <world>...` | Web-only full-screen dim, swallows clicks while the IndexedDB / cloud load is in flight; lobby returns early | Whole screen | W only | L2181-2186; draw_loading_overlay L1243 |
| H21 | Top strip `Your Signet contacts can't be read.` + `Disconnect` | `signet_contacts::disconnect()`; clears the host-refusal notice | Top panel, full width | N, Signet files unreadable AND width < 1360 | L2855-2877; signet_contacts/ui.rs:62 |
| H22 | `Played in the browser? Import your web worlds here.` | Hint text only (never auto-opens a dialog) | Above Trials tile | N, world list empty | L3022-3030 |

### 2.2 Trials tile and primary action row

| # | Label | What it does | Where | Shows when | file:line |
|---|---|---|---|---|---|
| T1 | Trials tile: coloured four-shelf band, `Trials`, `Little challenges. Pick one and play.`, `>` | `state.trials_gallery.open()`: the lobby is replaced by the full-screen Trials gallery (section 2.6) | Full row under header text; <=420 px wide x 78 px tall on >=900, else full width x 64 | ALL | L3035; trials_gallery_ui.rs:938-987 |
| P1 | `+ Create New World` (green, 200x40) | Sets `MenuDialog::Create{...defaults: survival, commands ON, stash OFF, normal, grass, depth 3, cycle, mobs ON}` | Action row, first | ALL | L3041-3062 |
| P2 | `🔧 The Workshop` (orange, 180x40) | `MenuAction::EnterWorkshop`: create-if-missing then load the single Workshop world (folder `WORKSHOP_FOLDER`), a void authoring space for reskinning blocks/mobs | Action row | ALL | L3063-3076; game_loop.rs:9372 |
| P3 | `♻ Reset` (tooltip "Clear everything built in The Workshop, back to an empty floor. Keeps your saved skins.") | Opens `MenuDialog::ResetWorkshop` confirm (section 2.6). On confirm: `MenuAction::ResetWorkshop` | Action row, right after Workshop | ALL (even if no Workshop exists yet) | L3077-3093 |
| P4 | `Join Game` (blue outline) | Opens `MenuDialog::JoinDirect` | Action row | N only | L3095-3109 |
| P5 | `Restore from file` | Web file picker -> `kick_off_import()` imports a single `.axeworld` into IndexedDB; status in P13 line | Action row | W only | L3110-3123; fn L1084 |
| P6 | `Export everything` (tooltip: "...saves one .axeprofile file with every world in this browser plus your Trials records.") | `kick_off_profile_export()` downloads a `.axeprofile`; disabled while running; "Packing your profile..." status | Action row | W only | L3125-3144; fn L1098 |
| P7 | `☁ Stash` (green explainer button) | Opens `MenuDialog::DesktopOnly(AppFeature::Stash)` popup pointing at the free desktop app | Action row | W only | L3148 |
| P8 | `🎮 Join` (green explainer) | `DesktopOnly(Join)` popup | Action row | W only | L3154 |
| P9 | `🖧 Host` (green explainer) | `DesktopOnly(Host)` popup | Action row | W only | L3160 |
| P10 | `Import world…` (tooltip "Open a single .axeworld file from anywhere on your computer") | Spawns OS file dialog (`FileDialogRequest::OpenWorld`); result handled in `poll_dialog` L596; status line "Opening the file picker..." | Action row | N; disabled while another dialog is open. **Android: always fails with "Worlds can't be imported or exported on this device yet."** | L3173-3190; native_file_dialog.rs:110 |
| P11 | `Import from folder` (tooltip "Reads every *.axeworld file in the world-transfer/ folder and imports each one") | Synchronous scan of `./world-transfer/*.axeworld`; refreshes list; status line | Action row (36 px tall, smaller than neighbours) | N | L3191-3207 |
| P12 | `Import a web profile…` (tooltip: "Open an .axeprofile file - every world you saved in the browser, plus your Trials records. Worlds already here are kept: a copy arrives as "(web)".") | **See section 4.** OS file dialog (`OpenProfile`) then `native_world_io::import_profile_native` | Action row (36 px) | N, always (not only first run). Android: fails like P10 | L3208-3231 |
| P13 | `Spawn: [ComboBox]` default `Default (saved position)` | **See section 4.** Sets `state.spawn_pref` | Own row under the action row (kept out of the wrap on purpose), combo width clamp 80..180 | ALL | L3234-3248 |
| P14 | Status line (transfer_status / import_status) e.g. "Saved to ...", "Imported 'X' from the file you picked", "Cancelled", "Save/open failed: ...", "Import failed: ..." | Label, 13 px green-grey, persists until the next import/export | Under spawn row | N: transfer_status; W: import_status ("Restored: name", "Packing your profile...") | L3250-3262 |
| P15 | Handover import lines "Waiting for <npub> to finish handing this world over" | One line per pending recipient handover (not a world card; cannot be opened until Activated) | Under status line | N, handover in progress | L3257-3262; field L267 |

### 2.3 World list

| # | Label | What it does | Where | Shows when | file:line |
|---|---|---|---|---|---|
| W0 | Empty state: brand mark (96 px), `No worlds yet`, `Tap "+ Create New World" to begin.` | Static | Centre of list area | list empty AND no pending handover imports. **On web this also flashes during the initial async IndexedDB load** (no `local_loaded` check). | L3277-3285 |
| W1 | World card collapsed: name (17 px bold), mode badge `SURVIVAL` / `CREATIVE`, `vN` (if version>0), publish badge, Stash dot, description or italic `No description`, `Last played: <relative>` (native only), size | Whole card click = select (expands); double-click = Play (`LoadWorld`). Keyboard/pad focusable with gold ring | Grid cell | Every world. Sorted by last played, newest first (native). | draw_world_card L3385-3470; click L3590-3615; sort save.rs:4386 |
| W2 | Publish badge `PUBLISHED` / `EDITS PENDING` | Display | Card header row | world has a publish record (`meta.publish_badge()`) | L3670-3694 |
| W3 | Stash dot 🔴/🟠/🟢 | Display: red = Stash off, amber = on but not confirmed uploaded, green = on and found in cloud manifest. **Native can never show green**: `stash_in_cloud` is hard-coded `false` on native (L3267-3270). | Card header, after badges | not on cloud-only entries (all native today: none exist) | L3418-3421, L3619-3627 |
| W4 | Mode badge | `SURVIVAL` / `CREATIVE` (anything not "creative" renders SURVIVAL; adventure/spectator are shown as SURVIVAL) | Card header | all | L3696-3712 |
| W5 | `▶ Play` / `▶ Play (from Stash)` (tooltip "Download this world from your Stash and play it") | `MenuAction::LoadWorld(folder, state.spawn_pref)`. First guard: `save::world_open_refusal` -> notice instead. | Expanded card, first | selected card. "(from Stash)" = `cloud_only` (web only) | L3485-3507; handle_card_action L1120 |
| W6 | `Host` | `MenuAction::HostWorld(folder)`. **See section 4.** Native: starts a Fray LAN room (room code on player's relays) and loads the world. **Web: the button still renders and just loads the world solo (same as Play, no hosting).** | Expanded card | selected; ALL (not cfg-gated) | L3513; game_loop.rs:9412 (native), 9473 (web) |
| W7 | `Host online` (disabled with reason tooltip) | `MenuAction::HostWorldOnline(folder)`. **See section 4.** | Expanded card | N, selected. Disabled when signed out ("Sign in with your Signet persona first - hosting needs an address."); enabled-with-note first time ("First time: one tap on your phone links this computer to you."). Re-read from disk every frame. | L3515-3529; friends_ui.rs:173; game_loop.rs:9854 |
| W8 | `Edit` | Opens `MenuDialog::Edit` (name + description). Refused for newer-version worlds. | Expanded card | selected | L3531, L1156 |
| W9 | `Fork` | `MenuDialog::Forking` -> synchronous `save::fork_world` copy then `RefreshWorlds`. **Web: renders but always fails: sets notice "fork_world not available on WASM".** | Expanded card | selected; ALL | L3532, L2438; save.rs:4587 |
| W10 | `Back up` | Downloads the packed world blob as a file (`export_world_wasm`) | Expanded card | W only | L3534-3535, L1180 |
| W11 | `Export…` | OS Save-As -> `.axeworld` (`export_dialog_request`); grey while another dialog open | Expanded card | N. Android: fails with "can't be imported or exported" message | L3543, L1184-1197 |
| W12 | `Export to folder` | Synchronous write `./world-transfer/<name>.axeworld`; status line | Expanded card | N | L3550, L3881 |
| W13 | `Stash: Off` / `Stash: On…` / `Stash: On` (coloured pill with dot; tooltips) | `CardAction::ToggleStash`: flips `meta.cloud_save` and persists `world_meta.json`. **See section 4.** | Expanded card, before Delete | N only; hidden for cloud-only | L3556-3573, L3632-3668, L1205 |
| W14 | `Delete` (red) | Opens `MenuDialog::Delete` (type-name-to-confirm) | Expanded card, last | selected | L3575-3583, L2398 |
| W15 | Card list content note | The singleton Workshop folder is excluded from the list. **Trial arena worlds (`exp-<slug>` folders) are NOT filtered** and appear as ordinary cards. Refused/damaged worlds still list with "(needs a newer version)" / "(can't be opened)" / "(world info damaged)" appended to the name and the reason as the description. | - | ALL | save.rs:4328-4368, 4447-4452 |

Selection: only one card is expanded at a time (`state.selected`). Expanded card shows a wrapping row of up to 9 buttons on native (Play, Host, Host online, Edit, Fork, Export…, Export to folder, Stash, Delete), 6 on web (Play, Host, Edit, Fork, Back up, Delete).

### 2.4 "The Stash" side panel (right, 264 px)

Width >= 1080, native AND web. Drawn by `draw_stash_column` (L1791-1937).

| # | Label | What it does | Shows when | file:line |
|---|---|---|---|---|
| S1 | `The Stash` title; lead `Browse an npub's open stash - play their experiences, adopt their skins.` | Label | width >= 1080 | L1832-1838 |
| S2 | Text field `Nostr address (npub)`, hint `npub1…` + `Browse` button (Enter also works) | **See section 4.** Web: fetches a Nostr Beacon listing for that npub; empty field = browse the official AxeNStax npub live. **Native: does nothing but print a status line.** | width >= 1080 | L1842-1862, L1939-1967 |
| S3 | Status line under input: `Browsing…`, `N item(s) found.`, `No experiences or skins in that stash.`, `Browse failed: …`, `That's not a valid npub or hex pubkey.`, `Fetching…`, `Download failed: …`, `Bad experience: …`, `That item has no download - skip.`; native: `Live browse is a PWA feature - the AxeNStax Official games below work offline.` / `Downloading a browsed item is a PWA feature.` | Label | after Browse/Play/Adopt | L1863-1866, 1950-1966, 1980-2012 |
| S4 | Section `AxeNStax Official` + `Always here, even offline.` | **See section 4.** Pinned list: Hash Dash, Satori Rush | always in column | L1870-1871; L261-275 |
| S5 | Official card: name, tag `Race` (Hash Dash) / `Speedrun` (Satori Rush), short author "AxeNStax Official", button `Play` | `MenuAction::PlayScenario{def}`: creates/enters a fresh per-def arena world and starts the scenario. Works with network off. | always | L1872-1883, 1724-1789; game_loop.rs:9634 |
| S6 | Section `From this stash` + browsed cards (name, tag `Experience` for undownloaded scenarios, author npub short) | Appears after a successful web Browse; official items deduped by name | W with results | L1886-1930 |
| S7 | Browsed card button `Play` (scenario) | Downloads the blob (web) then `PlayScenario` | W with results | L1756-1766, 1972-1990 |
| S8 | Browsed card button `✦ Adopt skin` (skin) | Downloads blob then `MenuAction::AdoptSkin{bytes}`: applied to the NEXT world entered (override registry lives on a World) | W with results | L1767-1780; game_loop.rs:9767 |

### 2.5 "Friends & servers" side panel (right, 248 px) - native only, width >= 1360

`friends_ui::draw_friends_column` (friends_ui.rs:395-470), one scroll area. Not mounted on web (no contacts book).

| # | Label | What it does | Shows when | file:line |
|---|---|---|---|---|
| F1 | `Your address`: signed out = `Sign in with your Signet persona to get an address friends can invite.` | Label | N signed out | friends_ui.rs:37-60 |
| F2 | `Your address`: QR (132 px), short npub, `Copy`, `Give this to a friend so they can invite you.` | Copy puts the npub on the clipboard | N signed in | friends_ui.rs:63-85 |
| F3 | `Friends` + `People you know, to build with.` | Label | always in column | friends_ui.rs:407-413 |
| F4 | Empty line `Nobody yet. Add a friend's address below, or join their world with an invite link.` | Label | no non-stranger contacts | friends_ui.rs:153, 416 |
| F5 | Contact rows: name or short npub (never hex) + small `Join` | `MenuAction::JoinContact{persona_hex, display_name}`. Join shown only if the tier admits play (kin/kith). Strangers never listed. | contacts exist | friends_ui.rs:427-440 |
| F6 | `Add a friend` | Opens `MenuDialog::AddFriend` (modal). Paste an npub (adds contact, `AddFriendNpub`) or an invite link (`JoinByInvite`, calls at once, adds nobody). | always in column | friends_ui.rs:444-449; menu.rs:2713-2765 |
| F7 | `Signet contacts` row: states Idle = lead `Use the people you already know in Signet here. Signet stays in charge of the list.` + `Connect Signet contacts`; Waiting = QR (200 px) + `Scan this with your phone, or open the link on it.` + `Copy link` + `Cancel` + `Waiting for Signet…`; Code = big 6-char code + `Type this code into Signet. Press Continue only when Signet says the code matches. If it doesn't, press Cancel.` + `Continue` / `Cancel`; TimedOut = `No answer from Signet. If you've connected lots of apps, Signet may be at its limit (10).` + `Try again` / `Cancel`; Connected = `Connected · N contacts · synced X ago` + `Sync now` + `Disconnect` + `Also remove "Axe'n'Stax" in Signet.`; extra lines `Your Signet list may be incomplete.`, `Your Signet list is out of date, so it isn't used until it syncs.`, `Your Signet contacts can't be read. Press Disconnect, then connect Signet again.`; notice line + `OK` (e.g. `Signet disconnected this game.`) | Pairing flow with Signet; `signet_contacts::start_pairing / continue_pairing / cancel_pairing / sync_now / disconnect`. No MenuAction. | native, width >= 1360 | signet_contacts/ui.rs:98-190 |
| F8 | `My Servers` + `Servers you've joined. Join by name - the address is looked up fresh.`; rows: ★ favourite, name or short npub, endpoint, `Join`, `Remove`; empty: `No saved servers yet. Join one with an axenstax://npub… link and it'll appear here.` | Join -> `MenuAction::JoinGame("axenstax://<npub>")`. Remove deletes from `my_servers` and saves (no confirm). | native, >= 1360 | menu.rs:1274-1385 |
| F9 | `My Keeps`; rows: name, age, `Join`, `Forget` (second tap `Forget for sure` + `Forget this Keep? You'll need a new invite to return.`) | Join -> `MenuAction::JoinKeep(invite)` (no confirm; already trusted). Forget removes from file. Hidden when empty. | native, >= 1360 (also inside Join Game dialog at any width) | menu.rs:1396-1500; keeps.rs:69 |

### 2.6 Modal dialogs and overlays reachable from the lobby

Dialogs drawn by `draw_main_menu` return early (the lobby underneath is not drawn) unless noted.

| # | Screen | Entry point | Contents (controls) | Platform | file:line |
|---|---|---|---|---|---|
| D1 | **Create New World** (`draw_create_dialog`) | P1 | Title `Create New World`; flavour line `Begin a new world in the Age of Diamonds.`; `WORLD NAME` field (hint "My New World"); `SEED` field (optional, text hashed to a seed); `GAME MODE`: `Survival` / `Creative`; `WORLD TYPE`: `Normal` / `Blank Canvas` / `🧪 Test Lab` (Test Lab native only; adds line "Satoshi will ask you to try new features and tap whether they work..."); if Blank Canvas: `GROUND` (None/Grass/Sand/Stone/Dirt/Snow/Water), `WATER DEPTH (blocks)` slider 1-8 (only if Water), `TIME` (Cycle/Day/Night), `MOBS` ON/OFF; `COMMANDS` ON/OFF; `SAVE TO YOUR STASH` `📦 ON`/`OFF` with line; buttons `Create & Play` (or `Create & Play (auto-name)` if name blank) / `Cancel`. Confirm writes the meta then `MenuAction::LoadWorld(folder, state.spawn_pref)`. | Stash toggle: N only | menu.rs:4115-4440; handler L2257-2355 |
| D2 | **Edit World** | W8 | `NAME`, `DESCRIPTION (optional)`, `Save` / `Cancel`; then `RefreshWorlds` | ALL | L4441-4503; L2357-2397 |
| D3 | **Delete confirm** | W14 | `This will permanently delete "<name>" and all its data.` / `This cannot be undone.` / `Type <name> to confirm`, field, `Delete Forever` (disabled until exact match; hint "Button enables when the name matches exactly") / `Cancel`; then `RefreshWorlds` (web chains async delete then re-list) | ALL | L4505-4616; L2398-2437 |
| D4 | **Forking** (no UI) | W9 | Pure state: copy happens in one frame, then `RefreshWorlds`; errors go to the notice | N works, W errors | L2438-2454 |
| D5 | **Join Game** (`draw_join_dialog`) | P4, or a held `axenstax:` server link | `GAMES ON THIS NETWORK` list (LAN beacon, click a room to join at once; `Looking for games on your Wi-Fi or Ethernet...`), `ROOM CODE` field (hint "KMT RWX") + help line `The host's code is in their pause menu. A Keep invite link or a server address (axenstax://...) works here too.`, `My Keeps` block (compact/phone: first, newest 3 + `More Keeps (N)` / `Fewer Keeps`; tall window: under the field, 190 px scroll), `Connect` / `Cancel`. `Connect` -> `MenuAction::JoinGame(text)`; picking a LAN room -> `JoinLanRoom`; Keep Join -> `JoinKeep`. Web variant (`SERVER ADDRESS`, hint 192.168.1.10:7700) exists in code but is unreachable from the web lobby because the web button opens D13 instead; only the dedicated-server page auto-join path uses `JoinGame` on web. | N | L4617-4772; L2455-2516; lan_ui.rs |
| D6 | **Join this Keep?** | A pasted/tapped Keep link whose Keep isn't in My Keeps (`launch_links` or Join text) | Keep label, `Only join a Keep from someone you trust. It will see who you are.`, `Join` (disabled until `CONFIRM_ARM` elapsed, so an in-flight tap can't accept it) / `Cancel` | N | L2517-2541; keeps.rs:50 |
| D7 | **Reset The Workshop?** | P3 | `This clears everything you've built in The Workshop and returns it to an empty floor.` / `Your saved skins and redesigns are kept.` / `Reset Workshop` (orange) / `Cancel`. Confirm -> `MenuAction::ResetWorkshop` | ALL | L2542-2575 |
| D8 | **Sign in with Signet** (Window) | H13 | Status-driven: AwaitingScan = `Scan this with the Signet app on your phone, then approve.` + QR (280 px, `nostrconnect://`) + spinner `Waiting for your phone…`; AwaitingApproval = `Connected - approve the sign-in in Signet on your phone.`; Success = `✓ Signed in as npub…` + `Done`; Failed = `Sign-in didn't complete: <msg>` + `Try again` / `Close`; Idle = `Starting…`. Always: `▸ Trouble scanning? Paste a bunker:// link` toggle -> field + `Connect with link`; `Signing in through: <relay hosts>`; `Cancel`; `Relays` (opens D9). On web the dialog closes itself instantly (never reachable). | N | L7011-7145; L2576-2598 |
| D9 | **Your relays** (Window) | D8 `Relays` button, or Settings | Relay manager: list, add field (`wss://` only, max 8, no dupes), Check (5 s websocket test), remove. Lead: `The app uses these to sign in, find friends, pair contacts and check for updates. Your game itself never runs through them.` Drawn by the caller over the lobby. | N | relays_ui.rs; game_loop.rs:9166 |
| D10 | **Settings / "Graphics"** (Window) | H15 | Texture pack, Render distance, Render scale, FOV, Mouse sensitivity, Brightness, Frame limit (VSync etc.), Particles (Off/Reduced/Full), Fog, Mipmaps, Leaves: Fast, Auto quality, Sound (Volume, Mute all sound), Third-person camera (distance, auto-centre speed, `Fade my avatar when the camera is close`), `Online` port field (N), Your relays (N), version line `AxeNStax v0.2.28`, pinned `Back`. Same panel as the pause menu. While open the lobby is blank behind it. | ALL (web: no Online / relays) | menu.rs:5865-6330 (draw), L5792-5840 (lobby plan); game_loop.rs:9187-9240 |
| D11 | **Add a friend** (Window) | F6, or a held friend-invite link | `Paste their address (npub) to add them, or an invite link to join their world.`, text field, error line, `Add` / `Cancel` | N, only reachable at >= 1360 or via an incoming link | L2713-2765; friends_ui.rs:217 |
| D12 | **Trials gallery** (full screen, replaces lobby) | T1 | `‹ Back`, title `Trials`, `Album`, `Walk the island`, `Pick one to look into it, then play.`, four colour shelves of trial cards, hero view per trial with `Play`, `How it works`, `Export ghost`, `Import ghost` (MenuAction `ExportGhost{trial_id}` / `ImportGhost`; `PlayScenario`). Modes: shelves, hero, album (postcards), island. Escape = back. | ALL | L2772-2790; trials_gallery_ui.rs:766-935, 540-760 |
| D13 | **Trial "How it works"** (egui Modal) | Gallery `How it works` | Trial icon + title, `<Type> trial`, `Pick a way:` route groups with numbered/bulleted tasks, `You'll craft: ...` line, `Cancel` / `Play` (`PlayScenario`); backdrop click / Esc = Cancel. Also drawn over the lobby proper. | ALL | menu.rs:1524-1662, 2778, 3374 |
| D14 | **Desktop-app explainer** (3 variants: Stash, Join, Host) | P7-P9 | Title (`☁  Stash - your private cloud` / `🎮  Multiplayer - join a world` / `🖧  Multiplayer - host a world`), lead, three ✦ bullets, italic CTA, `⬇  Get the free desktop app` (opens `/download` in a new tab), `Close`. Dim backdrop; lobby stays visible behind. Copy is legally pinned by `app_feature_copy_is_capability_framed`. | W only | L2599-2712, L89-150 |
| D15 | **World handover** (Window, top-centre) | Another player proposes a world handover, or a handover is in progress | Consent screen with `Accept` / `Decline`, progress text with `Cancel`, paused banner. Interactable on the lobby. | N | handover_game.rs:188-240; game_loop.rs:9144 |
| D16 | **OS file dialogs** | P10, P12, W11 | Native pickers (rfd); busy flag greys P10-P12 and W11/W12 | N (stub error on Android) | native_file_dialog.rs |

## 3. Notices (the lobby has ONE notice mechanism plus status lines; no true toasts)

`MenuState.notice` (banner H19, persists until OK) is set by:
- Refused world open/host/fork/edit/stash: newer-version, damaged, retired-by-handover world (menu.rs:1137 via `save::world_open_refusal`).
- Fork failure (L2447), world load failures `unopenable_message` (game_loop.rs:9035, 9087), "Local world load failed" on web.
- Disconnects, kicks, host closed, a Trial that couldn't open (`world_exit.rs:674, 744`).
- Join progress and errors: `Joining <label>...` (`lan_ui::joining_notice`), bad room code (`bad_code_notice`), `Sign in first to join a Keep.`, `That isn't a Keep invite link.` (game_loop.rs:9487-9542).
- Online host refusals: Signet unreadable (`HOST_REFUSED`, with a Disconnect Signet button), contacts-block errors (game_loop.rs:9859-9872).
- Handover notices (handover_game.rs:325, 878, 891).
- A queued launch (Trials / Workshop hop) that timed out: `QUEUED_LAUNCH_TIMED_OUT` (game_loop.rs:9299).
- Friend add / call failures (game_loop.rs:9880-10010).

There is also `GameState.toast` (bottom-centre green text) used by `online_toast(...)` for "Getting ready to play online…" (online_prep.rs:29) and host failures raised from lobby actions. `draw_toast` is only called from the pause overlay (game_loop.rs:10703) and in-world (21602), **not from the Menu branch**, so those toasts appear not to render while the player is still in the lobby (needs a runtime check). Host (LAN) toasts are queued via `entry_toast` and delivered after the world loads (L9454).

## 4. Specific explanations

**What `♻ Reset` next to The Workshop resets.** It resets the Workshop, not anything on the world list. Press -> confirm dialog (D7) -> `MenuAction::ResetWorkshop` -> `game_loop.rs:9372` enters the Workshop world exactly like the Workshop button (creating its meta if missing), sets `pending_workshop_reset`, and on the first play tick `reset_workshop_space()` (game_loop.rs:3607) calls `World::clear()` + `workshop.clear()`, regenerates the bedrock/void floor and forces an immediate autosave. Wiped: every block built in the Workshop and all in-progress reskin/reshape projects. Kept: `override_registry` (the saved skins/redesigns catalogue) and the player's skin wardrobe. Side effect: after Reset you are dropped INTO the Workshop. Reason it lives here: the Workshop is a singleton excluded from the world list (save.rs:4330), so it has no card and no Delete.

**What `Spawn: Default (saved position)` offers and applies to.** A single global dropdown (`state.spawn_pref`, not per-world; the Create dialog's code comment about a per-dialog override is stale, Create just uses this value). Options (spawn_pref.rs:57-69): `Default (saved position)`, `Near a village`, `Wilderness (no village in cell)`, `In open plains`, `In a forest`, `In the mountains`, `In the desert`, `On a beach`, `At origin (0, 0)`. Default = existing worlds resume at saved position, new worlds use the engine spawn. Others teleport the player on world entry (applied in chunk_stream.rs:648 then reset to Default). It rides ONLY on `MenuAction::LoadWorld` (Play button, card double-click, Create & Play). It is NOT applied to Host, Host online, Workshop, Trials / Stash scenarios (explicitly reset to Default, game_loop.rs:9389, 9676) or joins (reset at chunk_stream.rs:642). It is not saved between sessions (resets to Default each time the lobby is built) and gives no hint it is a one-shot setting.

**What `Stash: On…` on a world card toggles.** The button (native, expanded card only) flips the per-world `cloud_save` flag in that world's `world_meta.json` (`ToggleStash`, menu.rs:1205). Labels: `Stash: Off` (red, "stays on this computer"), `Stash: On…` (amber: opted in but not confirmed in the cloud), `Stash: On` (green: opted in and present in the cloud manifest). The ellipsis means "on, not confirmed uploaded". On native `stash_in_cloud` is hard-coded false (L3267-3270), so an opted-in native world can only ever show amber `Stash: On…`. Also, the engine's save path states "there is NO cloud upload here yet" (save.rs:4854-4856); I found no native consumer of `cloud_save` besides the pause menu toggle (game_loop.rs:10504-10545), so on native the toggle currently records intent only. Tooltip for amber says "Save the world (with a Stash sign-in) to upload it." The same flag is offered in Create (`SAVE TO YOUR STASH`) and in the in-world pause menu. Web has no such button (a web Stash is gated out; the `☁ Stash` button there is an explainer popup).

**`Host` vs `Host online`.**
- `Host` (`MenuAction::HostWorld`, native game_loop.rs:9412): starts a Fray host (`HostedServer::start_fray_host`, up to 4 remote players) and loads the world. Friends join by LAN beacon ("Games on this network" in Join Game) or a 6-letter room code (shown in the pause menu), with the room code announced on the player's own relays. Needs no sign-in beyond the Signet block list read. Delivers a toast after load telling how friends join. On web the button exists but just loads the world solo (game_loop.rs:9473).
- `Host online` (`MenuAction::HostWorldOnline`, game_loop.rs:9854): "online play by contact", hosting for friends in OTHER houses, reachable by their Signet persona/npub: runs `online_prep` (open a port / router mapping, mint a runtime-key attestation by a ONE phone tap on first use), then contacts call in via the Friends column `Join` or an invite link. Requires being signed in (disabled otherwise, with hover reason). Refuses if Signet contacts are unreadable. Native only.

**What `Import a web profile…` is.** Native-only button (always visible, 36 px) that opens an OS file picker for an `.axeprofile` file, the single bundle produced by the web lobby's `Export everything`. `import_profile_native` (native_world_io.rs:276) unpacks it: every bundled world becomes a world folder (never overwrites: a name clash arrives as `<name> (web)`), and the Trials records merge keeping the better of each. Status line e.g. `Imported 3 worlds · 1 kept alongside a world of the same name (renamed "(web)") · Trials records merged`. Purely local; no network/identity. It is the "take your worlds to native" path; the first-run hint H22 points at it but the hint itself doesn't say which button.

**What "The Stash" panel's `Browse an npub` does.** Input = a Nostr public key (npub1... or 64-hex). On WEB: fetches that key's public Beacon index (`open_stash::beacon_list`), keeps items whose content-type is a scenario ("experience") or skin, shows them under `From this stash` with `Play` (download blob, then `PlayScenario`) / `✦ Adopt skin` (download, then `AdoptSkin`, applied to the next world entered). Leaving the field empty and pressing Browse loads the official AxeNStax npub live. On NATIVE: nothing is fetched; it only prints `Live browse is a PWA feature - the AxeNStax Official games below work offline.` (menu.rs:1963-1966). Note this "Stash" is unrelated to the per-world cloud "Stash" toggle on world cards: two different features share the name. The panel is "open stash" content discovery by a key the player already knows (not a directory).

**What `AxeNStax Official` lists.** A pinned, embedded, zero-network list (menu.rs:261-275): two scenario cards, `Hash Dash` (tag `Race`) and `Satori Rush` (tag `Speedrun`; untimed first-Satori speedrun, resumable), each with a `Play` button that creates and enters a fresh arena world and starts the scenario. Defined in `assets/scenarios/hash-dash.json` and `satori-rush.json`. "Always here, even offline." A live browse of the same official npub is deduped against them by name.

## 5. Not covered by the `--shot-lobby` fixture

How the fixture works (lib.rs:3453-3560, `run_shot_lobby` / `sample_menu_state`): headless wgpu render of `draw_main_menu` twice (settle pass) at 3 sizes into `<out>/lobby-{desktop,tablet,phone}.png` (default dir `/tmp/lobby-shots`). Flags: `--empty` (clears worlds, so shows W0), `--gallery`, `--hero <key>`, `--album`, `--fixture-postcards`, `--fixture-unlocks fresh|started|several` (Trials gallery only). Fixed state: 5 made-up worlds ("My Castle" selected/expanded, "Sky City", "Cave Base", "Redstone Lab" with empty description, "Jungle Hideout"), mixed survival/creative, cloud_save on for 3 of them, `version` 1-47, no description on one. Separate: `--shot-signin [out]` renders D8 (AwaitingScan, QR) at 1280x800.

Not exercised by any fixture state:
- **Friends & servers column** (needs >= 1360; fixture max 1280). Also the Signet contacts states, My Servers, My Keeps, and the unreadable strip.
- Web-only UI: the whole web action row (Restore, Export everything, the three explainers, Back up on cards), web header (Exit), D14 popups, the loading veil. The fixture is native-only (`cfg(not(wasm32))`).
- Any dialog (Create, Edit, Delete, Join, Add friend, Reset, Keep confirm, handover window) and Settings, relays window.
- Notice banner (H19), the Signet strip, transfer/import status lines, awaiting-handover lines, stash sync line.
- Update-badge states: the fixture reads live `update_check::state()`, so it shows whatever the network gave (probably `Unknown` headless); Available/Update now/Restart/progress never forced. Same for sign-in: `host_online_state`, the identity line and Sign in/Sign out read the REAL profile on disk, so the shot differs by machine (guest vs signed in). The Stash column's `Browse` never runs.
- World-card states other than "selected = first card": cloud-only card, refused/damaged world cards, publish badges (`PUBLISHED` / `EDITS PENDING`), amber/green Stash, `Host online` disabled-vs-enabled, `Last played` is real time.
- Selected card on tablet/phone is shown (card 0 selected in all sizes), but the long wrapped action row's worst case is not forced.
- Compact dialog mode (Android / height < 620); only `menu_phone_panels_tests.rs` covers this, via unit tests.
- Empty list WITH pending handover imports; first-run nudge (visible with `--empty` on native).
- Long world names, many worlds (>5), 3-column layout (not reachable at 1280 with stash column = 2 cols).
- Trials tile in the hover/focus state; keyboard/pad focus rings.

## 6. World thumbnails or images today

None. `WorldEntry`/`WorldMeta` carry no image. `WorldMeta.icon: Option<String>` exists (save.rs:3567) but is always set `None` (save.rs:3802, 3894, 4530; menu.rs:716, 769, 896) and never read when drawing. The card is text only: name, badges, description, last played, size. The only imagery in the lobby is the wordmark (assets/brand/wordmark.png), the empty-state brand mark (`brand::show_mark`, 96 px), the Trials tile's four colour bars, and block-texture icons inside the Trials gallery. Trials gallery cards use procedural art (`paint_art`, trials_gallery_ui.rs:209), which is the only precedent for generated card imagery. Nothing renders a world preview/minimap in the lobby.

## 7. Design constraints that bind a redesign (from repo rules and tests)

- CLAUDE.md red lines: no AxeNStax-operated directory that lists player servers/worlds/groups; no "social network" framing; no money words; no collecting age/PII. The Friends column is deliberately "people you already know" (friends_ui.rs header comment; copy pinned by tests `the_online_copy_carries_no_discovery_money_or_social_words`, `the_copy_carries_no_social_discovery_or_money_words`, `app_feature_copy_is_capability_framed`, trials gallery `no_counts_or_money_words_in_the_gallery_copy` which also bans showing counts of trials).
- Web taster must stay login-free and anonymous.
- `world_page` is dead: the paginated grid was reverted to a vertical scroll on 2026-05-21 because kids missed the scroll affordance (menu.rs:300-305); compact dialogs use an always-visible scrollbar for the same reason.
- Controller / keyboard focus: custom-styled buttons need `focus_ring` (gold `brand::LANTERN`); collapsed cards are the focusable unit.
- Copy hygiene: "never hex, always npub" for identity display.
- Click-routing gotcha: an expanded card must NOT sense whole-card clicks (it ate every button press, menu.rs:3586-3596). Card Ids are salted by folder name.

## 8. Oddities / surprises (shortlist)

1. Web card shows `Host` (acts as Play) and `Fork` (always errors "fork_world not available on WASM").
2. Native `Browse` in The Stash does nothing but print "Live browse is a PWA feature".
3. Native Stash toggle can never go green, and I found no native upload code behind it.
4. Two different things are called "Stash" (per-world cloud flag vs. the npub-browse panel), plus a web-only `☁ Stash` explainer button.
5. `Spawn:` dropdown is global, one-shot-ish and ignores Host / Workshop / scenarios; Reset next to Workshop is the only Workshop-related action and is permanently visible.
6. Phone and tablet get no Stash panel, no Friends, no Add a friend, no Signet contacts; the Friends column only appears from 1360, so even 1280 desktop (the fixture) never shows it.
7. Trial arena worlds `exp-*` are not filtered from the list.
8. Native import controls are three near-identical blue buttons (`Import world…`, `Import from folder`, `Import a web profile…`) plus two export buttons per card (`Export…`, `Export to folder`); Android has stubs that only fail.
9. Update controls (`Update now`/`Restart`) only exist for Linux AppImage; Android/other native builds only ever get the badge + download link.
10. Lobby toasts (`online_toast`) likely invisible in the lobby; only the banner notice and status lines are drawn there.
11. `stash_sync_msg`, `world_page`, `stash_syncing` are dead state.
12. Web empty-list message flashes while IndexedDB loads.
13. Adventure/Spectator worlds render a `SURVIVAL` badge.

# Cloud Saves & the Stash Column

Your **Stash** is your worlds, backed up encrypted to the cloud so you can pick them up on any computer you sign in on. The **Stash Column** in the lobby is the other half of the idea: browse what someone else has put in *their* open Stash, and adopt their skins or play their Experiences.

This page is the reference for both. Cloud save needs you to be **signed in** (the same Signet sign-in you use to play) — everything is keyed to your own identity, and your worlds are encrypted so only you can open them.

> **Native app only.** Cloud saves and the Stash Column are a **native desktop app** feature. The web taster is a local-only sandbox — no login, no sync, no Stash toggle, no Stash Column. On the web build the Stash button just opens a "get the desktop app" explainer.

## How cloud save (the Stash) works

When a world is **stashed**, its save is encrypted on your computer and uploaded. Nobody — not us, not the storage server — can read it; only your signed-in identity holds the key. On another computer, sign in as the same player and your stashed worlds appear in the world list, ready to open.

Your Stash also follows you across devices for a couple of other things that ride the same plumbing:

- your **avatar skin** (see [Cosmetics & Skins](cosmetics.md)), and
- any **feedback** you send the makers, which sends at the same moment you sync.

### Turning Stash on for a world

Each world in your lobby has a **Stash traffic-light** button you can click:

| Light | Meaning |
|---|---|
| 🔴 **Stash: Off** | This world stays on this computer only. Click to start stashing it. |
| 🟠 **Stash: On…** | Stash is on, but this world isn't uploaded yet — save it (signed in) to upload. |
| 🟢 **Stash: On** | This world is in your Stash; open it on any computer. |

Toggling Stash **on** is per world — you choose which worlds get backed up.

### The Sync Stash button

The **📦 Sync Stash** button backs up every Stash-on world that isn't safely in the cloud yet, in one go. It's the deliberate "save my progress to the cloud" moment. While the game is open you'll also see a small **📦 Stashing…** toast in the top-right when a save uploads; it turns to **📦 Stashed — open it on any computer** when it's done.

> If you're offline or not signed in, your worlds still save **on your own device** — the cloud is a bonus, never a requirement. A failed upload always leaves your local save safe.

## The Stash Column

The lobby has three columns:

| Left | Middle | Right |
|---|---|---|
| Your worlds + the Workshop | **The Stash Column** | Friends / social |

The **middle column** lets you peek into another player's **open Stash** — the things they've chosen to publish — and act on each one.

### Browsing a Stash

Type (or paste) someone's **npub** into the column and press **Browse**. The column lifts what's in that npub's open Stash and lists it as cards, each tagged by type:

- **✦ Adopt skin** — a published Workshop skin or avatar skin. Adopt it to wear it.
- **▶ Play** — an **Experience** (a challenge or game mode someone has published). Play it to drop into it.

### AxeNStax Official

Pinned at the **top** of the column is an **AxeNStax Official** entry. It carries the two built-in Experiences — **Hash Dash** and **Satori Rush** — and it works with **no network at all** (they're built into the game), so they're always playable even if there's no wifi. A live browse of the official npub simply adds anything newer on top.

### Adopting a skin

Tap **Adopt** on a skin card and it downloads. A Workshop block-skin lands **in your Wardrobe, switched off** — open the Wardrobe (**K**) in the Workshop and **Set active** to wear it. An avatar skin is applied to your character. Adopting never overwrites your own active choice.

See [The Workshop](workshop.md) for the Wardrobe, and [Cosmetics & Skins](cosmetics.md) for avatar skins.

### Playing an Experience

Tap **Play** on an Experience card and it launches into its own **arena** — a fresh world made just for that challenge, so scores are fair. Your run saves to your **own** world; the published Experience stays a read-only template you can replay. The two official ones are covered on the [Minigames](minigames.md) page.

## Moving a world by hand (no cloud needed)

Cloud save isn't the only way to move a world about, and it isn't available on the web
build at all. Every world can also be written to a **file** you keep, copy to a memory
stick, or hand to a friend.

| Where | Button | What you get |
|---|---|---|
| **Web** (browser) | **Export** on a world card | `<name>.axeworld` — that one world |
| **Web** (browser) | **Export everything** | `<date>-axenstax-profile.axeprofile` — **all** your browser worlds *and* your Trials records |
| **Desktop app** | **Export** on a world card | `<name>.axeworld` — a Save dialog, you pick where |
| **Desktop app** | **Import world…** | opens one `.axeworld` file |
| **Desktop app** | **Import a web profile…** | opens one `.axeprofile` file |

### Taking your browser worlds to the desktop app

Worlds you build in the browser live **in that browser**, on that computer. If you want to
carry on in the free desktop app, you don't have to move them one at a time:

1. In the **browser** lobby, click **"Export everything"**. Your browser downloads a single
   `.axeprofile` file — every world you've saved here, plus your Trials times and
   completed challenges. (No worlds yet? It still exports your Trials records, and says so.)
2. Open the **desktop app** and click **"Import a web profile…"** in the lobby.
3. Pick the file you just downloaded. Your worlds appear in the list and your Trials
   records are merged in — for each trial you keep whichever time is **faster**, and a
   challenge you've completed anywhere stays completed.

**Nothing is ever overwritten.** If a world coming in has the same name as one already on
the desktop, the incoming copy is kept **alongside** it, named `<name> (web)`. You'll see a
line saying how many worlds arrived and how many had to be renamed.

> This is entirely a file on your own computer — a download, then a file picker. Nothing is
> uploaded anywhere, nothing is sent to us, and you don't need to be signed in.

## A note on identity

Everything here is tied to your **Nostr identity** (your npub) — the same one you sign in with. Player names are always shown as the npub, never raw keys. Your worlds are private (encrypted to you); a skin or Experience you *publish* is, by nature, public for others to adopt and play.

## See also

- [Cosmetics & Skins](cosmetics.md) — your avatar, and adopting skins.
- [The Workshop](workshop.md) — making block skins, and the Wardrobe.
- [Minigames](minigames.md) — Hash Dash & Satori Rush, whose records travel in an `.axeprofile`.
- [World Creation & Spawn](world-creation-and-spawn.md) — creating and loading worlds.

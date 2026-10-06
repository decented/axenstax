# Create an Art Gallery — a guide for artists

This is a step-by-step guide to turning your artwork into a **walkable gallery**
people explore in first person — wander the rooms, walk up to each piece, read the
label. You can run it as a quiet space to share with a link, or as a **kiosk** for
a real-world booth.

**You do not need to be technical.** You need to know how to make and save image
files (you already do), and a little about moving around the world. The one genuinely
techy step — running the gallery *server* — you can hand to a helper. More on that
below.

> **First time in the world?** Do the beginner lessons first so you're comfortable
> moving, looking around and using the hotbar:
> **[Getting Around](https://learn.axenstax.com/docs/journey/learn-journey/getting-around.md)**
> and **[Your First Blocks](https://learn.axenstax.com/docs/journey/learn-journey/first-blocks.md)**.
> This guide does **not** re-explain how to walk or place a block — it assumes you've
> had a quick play, or that you've got someone helping who has.

> **Not a gamer? Co-opt a young Minecrafter.** The building and the server bits are
> second nature to most Minecraft-savvy kids. Find one, point them at this guide and
> the **[Operator Guide](#stage-1--get-a-space-the-server-the-techy-bit)**, and ask for
> a hand for an afternoon (and keep a parent in the loop). You bring the art and the eye;
> they bring the world-building hands. It's a great split.

---

## What you'll end up with

- A set of rooms whose walls hold your art as **crisp, full-resolution pictures**
  (the deliberate contrast of sharp art on chunky blocks *is* the look).
- Optional **standing pieces** — free-standing cut-outs on a plinth that turn to
  face whoever walks up (perfect for sculptures or figures on a transparent
  background).
- A **label** under each piece (title / your name / a note).
- A link you can share so anyone walks your gallery **in their browser — no
  install**. Optionally a contained **kiosk** mode for a booth.

## The three stages (and who does what)

| Stage | What | Who |
|---|---|---|
| **1. A space (the server)** | The thing that hosts the world + the upload panel | **Helper / operator** (one-time, copy-paste Docker) |
| **2. Your art (upload)** | Get your pictures into the gallery | **You** — point-and-click in the Console |
| **3. Hanging it** | Put each piece on a wall, set size & labels | **You or your helper**, in the game |
| **4. Open the doors** | Kiosk mode + share the link | **You**, a couple of clicks |

Stages 2–4 are the artist's job and they're easy. Stage 1 is the techy one — read
on for how to get it done without becoming a sysadmin.

---

## The one thing to understand about size

You might worry about getting the **size or proportions** of your images right. **Don't.**

- Your picture **always hangs at its true shape** — it is **never squashed or
  stretched**. The gallery reads the real shape straight from your image file.
- The only thing you choose is **how big** the piece is on the wall, measured in
  **blocks** (one block ≈ one metre — about head height is 2 blocks).
- When you upload a picture, the Console hands you a **ready-made "Place command"**
  already sized to that picture's shape. Copy it, paste it in the game, done. No
  maths, no aspect ratios, no guessing.

So: **make your art whatever shape you like.** A tall portrait, a wide panorama, a
square — all hang correctly.

---

## Stage 1 — Get a space (the server) · *the techy bit*

A gallery people can visit needs a small **server** running somewhere. It provides
two things: the **world** visitors walk through, and an **Operator Console** (a
web page) where you upload your art.

This is the one step that isn't point-and-click. Two honest options:

1. **Get a helper.** Anyone comfortable with a terminal can stand it up in a few
   minutes — it's a copy-paste Docker setup. Hand them the
   **[Dedicated Server guide](https://docs.axenstax.org/docs/operators/dedicated-server.md)**
   and the **[Operator Console guide](https://docs.axenstax.org/docs/operators/operator-console.md)**.
   (This is the natural "ask a young Minecrafter for a hand" job.)
2. **Do it yourself** by following those two guides — they're written for
   non-experts and it's mostly pasting commands.

What you need from whoever sets it up:

- The **web address** of the gallery — either `https://<their-domain>` (a public
  gallery) or `https://<box-ip>:8443` (a home/LAN test; you click past a one-time
  certificate warning).
- To be added as an **operator** so you can use the Console: you'll **sign in with
  your Nostr / Signet identity** (the same one you sign into the game with), and the
  server's owner adds your **npub** to the operator list. Until they do, the Console
  will politely refuse you.

Once that's done, open `https://<your-gallery>/admin`, click **Sign in**, approve on
your phone/signer — you're in.

---

## Stage 2 — Upload your art · *you, point-and-click*

### Prepare your images (the part you already know)

- **Format:** **PNG** is best for crisp artwork and anything with transparency;
  **JPG** is fine and smaller for photographs. **GIF** and **WebP** also work.
- **Resolution:** upload at whatever resolution looks good to you — the gallery
  shows it at full quality. A long edge of roughly **1500–3000 px** looks great.
  **Keep each file under 12 MB.**
- **Transparent cut-outs:** if you want a **standing** piece (a figure or sculpture
  that stands on a plinth), export a **PNG with a transparent background** — the
  gallery cuts it out cleanly so it isn't a floating rectangle.
- **Shape:** anything. Portrait, landscape, square, panorama — it'll hang true.

### Upload them

1. Open the Console: `https://<your-gallery>/admin`, sign in.
2. Go to the **Gallery / Studio** panel.
3. Click **Upload**, choose an image. It appears in the list straight away, showing
   its **pixel size** and a **Copy place command** button.
4. Repeat for every piece.

> Tip: give your files **simple, lowercase names** before uploading — `sunrise.jpg`,
> `figure-01.png`. The Console tidies names automatically (spaces and odd characters
> are removed), so `My Painting!.jpg` becomes `MyPainting.jpg` — just so you know
> what to type if you ever reference it by hand.

---

## Stage 3 — Hang your art in the world · *you or your helper*

A piece can hang two ways:

- **Wall** — flat against a wall, like a framed painting; faces the room.
- **Standing** — a free-standing piece on the floor or a plinth that always turns to
  face the viewer (use a transparent PNG).

### The easy way — paste the Place command

1. In the game, **look at the wall** where you want the piece (or the floor/plinth,
   for a standing piece).
2. Press **T** to open the chat.
3. **Paste the Copy place command** you got in the Console, press **Enter**.

The picture appears, at its true shape. That's it.

The command looks like:

```
/exhibit place sunrise.jpg wall 4 2.25
```

…which means *"hang `sunrise.jpg` on the wall I'm looking at, about 4 blocks wide."*
The two numbers are just the **size box** in blocks — make them bigger or smaller to
taste; **the shape always stays correct** (if the box isn't the same shape as the
picture, the picture simply sits neatly inside it rather than distorting).

> **Handing it to a helper?** Just send them your list of Copy commands. They look at
> each wall and paste — no curating decisions needed, you've already made them.

### Standing pieces

Same idea, with `standing` instead of `wall`, looking at the floor/plinth:

```
/exhibit place figure-01.png standing 2 3
```

### Tidy up after placing

All of these are typed in the chat (press **T** first). Each piece has a number —
run `/exhibit list` to see them.

> **Shortcuts:** `/exhibit` also answers to the shorter `/ex`. `wall`/`standing` can be
> typed as `w`/`s`/`stand`, and `delete` also answers to `remove`/`del`.

| Command | What it does |
|---|---|
| `/exhibit list` | List every placed piece with its number |
| `/exhibit resize <n> <w> <h>` | Make piece **n** bigger/smaller (shape stays true) |
| `/exhibit move <n>` | Look at a new spot, then run this to move piece **n** there |
| `/exhibit yaw <n> <degrees>` | Turn a **wall** piece to a different angle |
| `/exhibit label <n> <text>` | Add a title / credit plaque under the piece |
| `/exhibit image <n> <file>` | Swap which picture a piece shows |
| `/exhibit delete <n>` | Remove a piece |

> **Why this needs "building power."** Placing and editing exhibits is a creative
> building tool, so it's available in your **own creative world** and to **operators**
> on a shared server — not to ordinary visitors (so nobody can rearrange your show).
> If you can't place a piece on a shared server, ask the owner to grant you operator
> power, or build the gallery in your own creative world first (see below).

### The recommended way to build the whole thing

The smoothest workflow, especially with a helper:

1. Build the gallery **in a creative world** first — walls, rooms, plinths, and all
   the exhibits placed exactly how you want them. You see it exactly as visitors
   will, as you build.
2. Your helper/operator then **publishes** that world to the server (a one-step copy
   — it's in the [Operator Guide](https://docs.axenstax.org/docs/operators/dedicated-server.md)).
3. From then on, use the **Console** to add or swap pictures on the live gallery.

---

## Stage 4 — Open the doors · *you, a couple of clicks*

- **Let visitors in.** Servers require sign-in by default, and a web browser can't
  sign in, so pick **Anyone** in the setup wizard's *Who can come in?* step (or untick
  *Require sign-in* in the Console's Access panel).
- **Just share it.** Anyone you give the web address to can walk the gallery in their
  browser. Exhibits show up for every visitor automatically.
- **Kiosk / booth mode (optional).** In the Console's **World / Showcase** panel,
  turn **Showcase** on. Visitors then explore in a contained, read-only mode — they
  can't break anything, **clicking a piece "collects" it** into a little basket, and
  leaving lands on a clean exit screen instead of dumping them elsewhere. Ideal for a
  gallery booth or a public link. (Your helper can also set this with the server's
  `AXENSTAX_SHOWCASE` switch.)
- **Share the link.** `https://<your-gallery>` — that's all anyone needs.

---

## Selling & tips

There is no checkout in the game: AxeNStax does not sell prints or handle any
payment for you. To point visitors at your own shop or tip link, put it in the
piece's **label** (`/exhibit label <n> Prints: yourshop.com`).

---

## If something looks wrong

| What you see | What it means / what to do |
|---|---|
| A **blank panel** where art should be | The image file isn't on the server. Re-upload it in the Console; if your helper copied a world over, make sure the `exhibits` folder of images went with it. |
| The picture looks **too small in its frame** | The size box is a different shape from the picture, so it sits neatly inside. Use the Console's **Copy place command** (it sizes the box to match the picture exactly), or tweak the two numbers. It is **never** stretched. |
| **"You can't do that"** when placing | You need building/operator power on that server — ask the owner, or build in your own creative world and have it published. |
| The **Console won't let you in** | The server owner needs to add your npub to the operator list. |

---

## Quick reference — the `/exhibit` command

Open the chat with **T**, then:

```
/exhibit place <image> <wall|standing> [width] [height]   place a new piece
/exhibit list                                             list placed pieces + numbers
/exhibit resize <n> <width> <height>                      change a piece's size box
/exhibit move <n>                                         move piece n to where you're looking
/exhibit yaw <n> <degrees>                                turn a wall piece
/exhibit label <n> <text>                                 set the plaque text
/exhibit image <n> <file>                                 swap the picture
/exhibit delete <n>                                       remove a piece
```

Sizes are in **blocks** (≈ metres). Pictures always keep their true shape.

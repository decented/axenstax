//! One-shot right-click interactions with a mob (MP-D2b): breeding feed,
//! taming, shearing, milking, a Lead on and off, and a pet's sit / follow
//! command.
//!
//! **One rule, two callers.** Single-player's right-click (`game_loop`) runs
//! these on its own ECS for a local player; a server runs them on ITS ECS for
//! a joiner's validated `EntityInteract` (`hosted_server`), so a joiner and a
//! single-player player feed, tame and shear by the same code. Each function
//! checks its own preconditions and returns `None` when it does not apply
//! (single-player falls through to the next right-click branch; a server
//! refuses the request). What the caller does with the result is its own
//! business: single-player takes `consume` from the held stack and adds
//! `give` to the inventory; a server tells the joiner to take `consume`
//! (`InteractOutcome`) and grants `give` through `InventoryGrant`. Toasts,
//! particles, audio and challenge events stay with the client.
//!
//! **Who acts** is an [`Actor`]: its pet-owner key (`"local-player-{slot}"`
//! for a local seat, the verified npub for a joiner — `None` for a guest, who
//! cannot tame) and where a Lead it holds is fastened.

use crate::entity::Position;
use crate::item::{Item, ItemStack, MaterialId};
use crate::mob::MobType;
use crate::scenario::ChallengeEvent;
use crate::tether::{TetherTarget, Tethered};

pub use crate::protocol::InteractKind;

/// Who is interacting.
#[derive(Clone, Copy, Debug)]
pub struct Actor<'a> {
    /// The actor's pet-owner key; `None` = a guest joiner (no verified npub),
    /// who cannot tame (a pet must have an owner that outlives the session).
    pub owner_key: Option<&'a str>,
    /// What a Lead this actor attaches (or a freshly tamed pet's auto-leash)
    /// is fastened to: the actor's player slot.
    pub tether: TetherTarget,
    /// Who the actor is, as a kill or a breed credits it: a local player's
    /// slot, or a joiner's slot + connection generation. Recorded on an
    /// animal it feeds (`breeding::InLove::fed_by`, review D2b B2).
    pub who: crate::combat::Attacker,
}

/// What the player is told about an interaction. Wire-stable codes
/// (`InteractOutcomePacket.note`), append only; 0 = nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum InteractNote {
    None = 0,
    Fed = 1,
    Milked = 2,
    MilkNotReady = 3,
    Sheared = 4,
    WoolGrowing = 5,
    Tamed = 6,
    TameWary = 7,
    AlreadyTamed = 8,
    Sat = 9,
    Rose = 10,
    CompanionFollow = 11,
    CompanionStay = 12,
    CompanionWander = 13,
    CompanionPerch = 14,
    SignInToTame = 15,
    NotYourPet = 16,
    /// Review D2b MEDIUM-2 — the server does not simulate what this needs
    /// (breeding, Leads, pets following): refused, nothing used.
    NotOnThisServer = 17,
}

impl InteractNote {
    pub fn to_wire(self) -> u8 {
        self as u8
    }

    /// An unknown code (a newer server) reads as nothing.
    pub fn from_wire(code: u8) -> Self {
        use InteractNote::*;
        match code {
            1 => Fed,
            2 => Milked,
            3 => MilkNotReady,
            4 => Sheared,
            5 => WoolGrowing,
            6 => Tamed,
            7 => TameWary,
            8 => AlreadyTamed,
            9 => Sat,
            10 => Rose,
            11 => CompanionFollow,
            12 => CompanionStay,
            13 => CompanionWander,
            14 => CompanionPerch,
            15 => SignInToTame,
            16 => NotYourPet,
            17 => NotOnThisServer,
            _ => None,
        }
    }

    /// The toast for this note on a `kind` mob (`None`: no mob — a Lead on a
    /// fence post), and how long it shows (seconds); `first_tame_hint` = this
    /// is the session's first tame, whose toast teaches the pet-command
    /// gesture. UK English; the single-player wording, verbatim.
    pub fn toast(self, kind: Option<MobType>, first_tame_hint: bool) -> Option<(String, u64)> {
        use InteractNote::*;
        let wolf = kind == Some(MobType::Wolf);
        let nostrich = kind == Some(MobType::Nostrich);
        let name: &str = kind.map_or("It", |k| &crate::mob::mob_def(k).name);
        let s = |t: &str, secs: u64| Some((t.to_string(), secs));
        match self {
            None => Option::None,
            Fed => s("Fed! Bring another to breed.", 2),
            Milked => s("Milked the cow.", 2),
            MilkNotReady => s("This cow needs time before it can be milked again.", 2),
            Sheared => s("Sheared the sheep.", 2),
            WoolGrowing => s("This sheep's wool is still growing back.", 2),
            Tamed if first_tame_hint => {
                s("Tamed! Right-click it with an empty hand to set Follow / Stay / Wander.", 5)
            }
            Tamed if wolf => s("Tamed! The wolf is yours.", 4),
            Tamed if nostrich => s("Tamed! The Nostrich is yours.", 4),
            Tamed => s("Tamed! It's your companion now.", 3),
            TameWary if wolf => s("The wolf takes the bone, eyes wary…", 2),
            TameWary if nostrich => s("The Nostrich eyes you carefully…", 2),
            TameWary => s("It sniffs the food, still wary…", 2),
            AlreadyTamed if wolf => s("This wolf is already yours.", 2),
            AlreadyTamed if nostrich => s("This Nostrich is already tame.", 2),
            AlreadyTamed => Option::None,
            Sat if nostrich => s("Sit. The Nostrich settles.", 2),
            Sat => s("Sit. The wolf settles.", 2),
            Rose if nostrich => s("Follow. The Nostrich rises.", 2),
            Rose => s("Follow. The wolf rises.", 2),
            CompanionFollow => companion_toast(name, crate::companion::CompanionState::Follow),
            CompanionStay => companion_toast(name, crate::companion::CompanionState::Stay),
            CompanionWander => companion_toast(name, crate::companion::CompanionState::Wander),
            CompanionPerch => Some((format!("{name}: Perch (hops to your shoulder)"), 2)),
            SignInToTame => s("Sign in to tame animals in someone else's world.", 3),
            NotYourPet => s("That's not your pet.", 2),
            NotOnThisServer => s("This server doesn't support that yet.", 3),
        }
    }

    fn companion(state: crate::companion::CompanionState) -> Self {
        use crate::companion::CompanionState;
        match state {
            CompanionState::Follow => InteractNote::CompanionFollow,
            CompanionState::Stay => InteractNote::CompanionStay,
            CompanionState::Wander => InteractNote::CompanionWander,
            CompanionState::Perch => InteractNote::CompanionPerch,
        }
    }
}

fn companion_toast(name: &str, state: crate::companion::CompanionState) -> Option<(String, u64)> {
    Some((format!("{name}: {}", crate::companion::state_label(state)), 2))
}

/// What an interaction did.
#[derive(Clone, Debug, PartialEq)]
pub struct Interaction {
    /// It happened (a refusal with a note — "the cow needs time", "sign in to
    /// tame" — is `false`).
    pub done: bool,
    /// Items to take from the held stack.
    pub consume: u8,
    /// Products for the actor's inventory (a milk bucket; a Lead taken off).
    /// Shorn wool is NOT here: it drops at the sheep as a world item, as in
    /// single-player, and is picked up like any drop.
    pub give: Vec<ItemStack>,
    pub note: InteractNote,
    /// The scenario event the actor's client fires.
    pub challenge: Option<ChallengeEvent>,
    /// A tame landed (the client's poof and pet-command hint).
    pub tamed: bool,
}

impl Interaction {
    fn done(consume: u8, note: InteractNote) -> Self {
        Self { done: true, consume, give: Vec::new(), note, challenge: None, tamed: false }
    }

    pub fn refused(note: InteractNote) -> Self {
        Self { done: false, consume: 0, give: Vec::new(), note, challenge: None, tamed: false }
    }
}

fn held_material(held: Option<&Item>) -> Option<MaterialId> {
    match held {
        Some(Item::Material(m)) => Some(*m),
        _ => None,
    }
}

/// The deterministic tame-roll seed single-player has always used.
fn tame_seed(target: hecs::Entity, tick: u64) -> u64 {
    tick.wrapping_mul(374_761_393).wrapping_add(target.id() as u64 * 668_265_263)
}

/// A tamed pet is no longer disposable wildlife: leashed to its new owner,
/// and without `Scattered`, so chunk unload never despawns it.
fn settle_new_pet(ecs: &mut hecs::World, target: hecs::Entity, tether: Option<TetherTarget>) {
    if let Some(t) = tether {
        let _ = ecs.insert_one(target, Tethered { target: t });
    }
    let _ = ecs.remove_one::<crate::entity::Scattered>(target);
}

/// P5 — an adult's breeding food puts it in love mode (two in-love adults of
/// a species pair into a baby on the breeding tick). The horse family only
/// while sneaking (plain right-click mounts it). Not a baby, not on its
/// breeding cooldown, not already in love. `fed_by` is recorded on the
/// animal, so the baby credits its feeder (review D2b B2).
pub fn feed(
    ecs: &mut hecs::World,
    target: hecs::Entity,
    kind: MobType,
    held: Option<&Item>,
    sneak: bool,
    fed_by: crate::combat::Attacker,
    tick: u64,
) -> Option<Interaction> {
    let mat = held_material(held)?;
    let applies = crate::breeding::breeding_food(kind) == Some(mat)
        && crate::breeding::breeding_feed_allowed(kind, sneak)
        && ecs.get::<&crate::breeding::Baby>(target).is_err()
        && ecs.get::<&crate::breeding::BreedCooldown>(target).is_err()
        && ecs.get::<&crate::breeding::InLove>(target).is_err();
    if !applies {
        return None;
    }
    let _ = ecs.insert_one(
        target,
        crate::breeding::InLove {
            until_tick: tick + crate::breeding::LOVE_DURATION_TICKS,
            fed_by: Some(fed_by),
        },
    );
    Some(Interaction::done(1, InteractNote::Fed))
}

fn product_ready(ecs: &hecs::World, target: hecs::Entity, tick: u64) -> bool {
    ecs.get::<&crate::animal_products::AnimalProductState>(target)
        .map(|s| crate::animal_products::can_milk(*s, tick))
        .unwrap_or(false)
}

fn record_product(ecs: &mut hecs::World, target: hecs::Entity, tick: u64) {
    if let Ok(mut s) = ecs.get::<&mut crate::animal_products::AnimalProductState>(target) {
        crate::animal_products::record_action(&mut s, tick);
    }
}

/// Animals Wave 2 — a bucket on a cow: the bucket becomes a milk bucket.
/// A cow milked recently isn't ready (refused, nothing consumed).
pub fn milk(
    ecs: &mut hecs::World,
    target: hecs::Entity,
    kind: MobType,
    held: Option<&Item>,
    tick: u64,
) -> Option<Interaction> {
    if kind != MobType::Cow || held_material(held) != Some(MaterialId::Bucket) {
        return None;
    }
    if !product_ready(ecs, target, tick) {
        return Some(Interaction::refused(InteractNote::MilkNotReady));
    }
    record_product(ecs, target, tick);
    let mut done = Interaction::done(1, InteractNote::Milked);
    done.give.push(ItemStack::new_material(MaterialId::MilkBucket, 1));
    done.challenge = Some(ChallengeEvent::ShearOrMilk);
    Some(done)
}

/// Animals Wave 2 — shears on a sheep: 1-3 wool (+0-3 for a high-yield
/// bred sheep) drop at the sheep. Shears don't wear. A sheep sheared
/// recently is still growing its wool (refused).
pub fn shear(
    ecs: &mut hecs::World,
    target: hecs::Entity,
    kind: MobType,
    held: Option<&Item>,
    tick: u64,
) -> Option<Interaction> {
    let holding_shears = matches!(
        held,
        Some(Item::Tool(t)) if t.tool_type == crate::crafting::ToolType::Shears
    );
    if kind != MobType::Sheep || !holding_shears {
        return None;
    }
    if !product_ready(ecs, target, tick) {
        return Some(Interaction::refused(InteractNote::WoolGrowing));
    }
    let pos = ecs.get::<&Position>(target).map(|p| p.0).ok()?;
    // W1B — genetics payoff: a high-yield_q (selectively bred) sheep grows
    // more wool. Baseline (128) → +1, champion (255) → +3, poor (<80) → +0.
    let yield_q = ecs
        .get::<&crate::genetics::Genetics>(target)
        .map(|g| g.yield_q)
        .unwrap_or(128);
    let bonus = (yield_q as u32 / 80) as u8; // 0..=3
    let n = 1 + (tick as u32 % 3) as u8 + bonus; // 1..=6 wool
    let wool = ItemStack::new_material(MaterialId::Wool, n);
    crate::entity::spawn_item(ecs, pos, wool, tick as u32);
    record_product(ecs, target, tick);
    let mut done = Interaction::done(0, InteractNote::Sheared);
    done.challenge = Some(ChallengeEvent::ShearOrMilk);
    Some(done)
}

/// Companions wave — a Cat / Parrot / Fox (…) right-clicked with its food: a
/// tame roll (a Cat Treat on a Cat always tames). The food goes either way.
/// An already-tamed companion doesn't take it (`None`: the click falls
/// through, as in single-player).
pub fn tame_companion(
    ecs: &mut hecs::World,
    target: hecs::Entity,
    kind: MobType,
    held: Option<&Item>,
    actor: &Actor,
    tick: u64,
) -> Option<Interaction> {
    let mat = held_material(held)?;
    let is_treat = mat == MaterialId::CatTreat && kind == MobType::Cat;
    if !is_treat && crate::companion::tame_food(kind) != Some(mat) {
        return None;
    }
    let tamed_already = ecs
        .get::<&crate::companion::CompanionData>(target)
        .map(|d| d.ownership.is_tamed())
        .unwrap_or(true);
    if tamed_already {
        return None;
    }
    let Some(key) = actor.owner_key else {
        return Some(Interaction::refused(InteractNote::SignInToTame));
    };
    let outcome = {
        let mut data = ecs.get::<&mut crate::companion::CompanionData>(target).ok()?;
        if is_treat {
            crate::tameable::attempt_tame_guaranteed(&mut data.ownership, key)
        } else {
            crate::tameable::attempt_tame_generic(
                &mut data.ownership,
                key,
                tame_seed(target, tick),
                crate::companion::TAME_NUMER,
                crate::companion::TAME_DENOM,
            )
        }
    };
    match outcome {
        crate::tameable::TameAttempt::Succeeded => {
            // A companion follows by its own dispatch; no auto-leash.
            settle_new_pet(ecs, target, None);
            let mut done = Interaction::done(1, InteractNote::Tamed);
            // Task 18 — companion tames count for Trials too.
            done.challenge = Some(ChallengeEvent::TameMob);
            done.tamed = true;
            Some(done)
        }
        crate::tameable::TameAttempt::Failed => Some(Interaction::done(1, InteractNote::TameWary)),
        crate::tameable::TameAttempt::AlreadyTamed => None,
    }
}

/// Spec 28d.wolves R5 — a Bone on a wolf: a tame roll (the bone goes unless
/// the wolf is already tamed); success leashes the wolf to its new owner.
pub fn tame_wolf(
    ecs: &mut hecs::World,
    target: hecs::Entity,
    held: Option<&Item>,
    actor: &Actor,
    tick: u64,
) -> Option<Interaction> {
    if held_material(held) != Some(MaterialId::Bone) {
        return None;
    }
    let tamed_already = ecs.get::<&crate::wolf::WolfData>(target).ok()?.ownership.is_tamed();
    if tamed_already {
        return Some(Interaction::refused(InteractNote::AlreadyTamed));
    }
    let Some(key) = actor.owner_key else {
        return Some(Interaction::refused(InteractNote::SignInToTame));
    };
    let outcome = {
        let mut data = ecs.get::<&mut crate::wolf::WolfData>(target).ok()?;
        crate::wolf::attempt_tame(&mut data, key, tame_seed(target, tick))
    };
    match outcome {
        crate::wolf::TameOutcome::Succeeded => {
            settle_new_pet(ecs, target, Some(actor.tether));
            let mut done = Interaction::done(1, InteractNote::Tamed);
            done.challenge = Some(ChallengeEvent::TameMob);
            done.tamed = true;
            Some(done)
        }
        crate::wolf::TameOutcome::Failed => Some(Interaction::done(1, InteractNote::TameWary)),
        crate::wolf::TameOutcome::AlreadyTamed => {
            Some(Interaction::refused(InteractNote::AlreadyTamed))
        }
    }
}

/// Spec 28d.nostrich v2 — Mixed Berries on a Nostrich: a tame roll (the
/// berry goes unless already tamed); success leashes it to its new owner.
pub fn tame_nostrich(
    ecs: &mut hecs::World,
    target: hecs::Entity,
    held: Option<&Item>,
    actor: &Actor,
    tick: u64,
) -> Option<Interaction> {
    let mat = held_material(held);
    if mat != Some(MaterialId::Berries) {
        return None;
    }
    let tamed_already = ecs.get::<&crate::nostrich::NostrichData>(target).ok()?.is_tamed();
    if tamed_already {
        return Some(Interaction::refused(InteractNote::AlreadyTamed));
    }
    let Some(key) = actor.owner_key else {
        return Some(Interaction::refused(InteractNote::SignInToTame));
    };
    let outcome = {
        let mut data = ecs.get::<&mut crate::nostrich::NostrichData>(target).ok()?;
        crate::nostrich::attempt_tame_with_berry(&mut data, mat, key, tame_seed(target, tick))
    };
    use crate::nostrich::BerryFeedOutcome;
    match outcome {
        BerryFeedOutcome::Tamed => {
            settle_new_pet(ecs, target, Some(actor.tether));
            let mut done = Interaction::done(1, InteractNote::Tamed);
            done.tamed = true;
            Some(done)
        }
        BerryFeedOutcome::BerryConsumed => Some(Interaction::done(1, InteractNote::TameWary)),
        BerryFeedOutcome::AlreadyTamed => Some(Interaction::refused(InteractNote::AlreadyTamed)),
        BerryFeedOutcome::NotAFeed => None,
    }
}

/// The species' tame, whichever it is (a joiner's `InteractKind::Tame`).
pub fn tame(
    ecs: &mut hecs::World,
    target: hecs::Entity,
    kind: MobType,
    held: Option<&Item>,
    actor: &Actor,
    tick: u64,
) -> Option<Interaction> {
    match kind {
        MobType::Wolf => tame_wolf(ecs, target, held, actor, tick),
        MobType::Nostrich => tame_nostrich(ecs, target, held, actor, tick),
        k if crate::companion::is_companion_species(k) => {
            tame_companion(ecs, target, k, held, actor, tick)
        }
        _ => None,
    }
}

/// Empty hand on the actor's OWN pet: a wolf or Nostrich sits / follows; a
/// companion steps through its command cycle (Follow / Stay / Wander, and
/// Perch for a parrot). Someone else's pet: `NotYourPet` (refused); a wild
/// mob or a hand that isn't empty: `None`.
pub fn sit_toggle(
    ecs: &mut hecs::World,
    target: hecs::Entity,
    kind: MobType,
    held: Option<&Item>,
    actor: &Actor,
) -> Option<Interaction> {
    if held.is_some() {
        return None;
    }
    let owner = crate::tameable::pet_owner_of(ecs, target)?;
    let key = actor.owner_key.unwrap_or("");
    if key.is_empty() || owner != key {
        return Some(Interaction::refused(InteractNote::NotYourPet));
    }
    match kind {
        MobType::Nostrich => {
            let mut data = ecs.get::<&mut crate::nostrich::NostrichData>(target).ok()?;
            crate::nostrich::toggle_sit_follow(&mut data);
            let note = match data.state {
                crate::nostrich::NostrichAiState::Sit => InteractNote::Sat,
                _ => InteractNote::Rose,
            };
            Some(Interaction::done(0, note))
        }
        MobType::Wolf => {
            let mut data = ecs.get::<&mut crate::wolf::WolfData>(target).ok()?;
            let state = data.try_toggle_sit(key)?;
            let note = match state {
                crate::wolf::WolfAiState::Sit => InteractNote::Sat,
                _ => InteractNote::Rose,
            };
            Some(Interaction::done(0, note))
        }
        k if crate::companion::is_companion_species(k) => {
            let mut data = ecs.get::<&mut crate::companion::CompanionData>(target).ok()?;
            let next = crate::companion::cycle_state(data.state, crate::companion::can_perch(k));
            data.state = next;
            Some(Interaction::done(0, InteractNote::companion(next)))
        }
        _ => None,
    }
}

/// Spec 36 — a Lead on a passive mob fastens it to the actor (replacing any
/// tether it had). The Lead is used up.
pub fn lead_attach(
    ecs: &mut hecs::World,
    target: hecs::Entity,
    kind: MobType,
    held: Option<&Item>,
    actor: &Actor,
) -> Option<Interaction> {
    if held_material(held) != Some(MaterialId::Lead)
        || crate::mob::mob_def(kind).category != crate::mob::MobCategory::Passive
    {
        return None;
    }
    let _ = ecs.remove_one::<Tethered>(target);
    let _ = ecs.insert_one(target, Tethered { target: actor.tether });
    Some(Interaction::done(1, InteractNote::None))
}

/// Spec 36 — a right-click on a tethered mob without a Lead in hand takes the
/// Lead off (anyone's: a Lead is not ownership) and gives it back.
pub fn lead_detach(
    ecs: &mut hecs::World,
    target: hecs::Entity,
    held: Option<&Item>,
) -> Option<Interaction> {
    if held_material(held) == Some(MaterialId::Lead) || ecs.get::<&Tethered>(target).is_err() {
        return None;
    }
    let _ = ecs.remove_one::<Tethered>(target);
    let mut done = Interaction::done(0, InteractNote::None);
    done.give.push(ItemStack::new_material(MaterialId::Lead, 1));
    Some(done)
}

/// Spec 36 — a Lead on a fence post: the actor's leashed mob nearest the
/// post (within [`POST_TRANSFER_RADIUS`] blocks, horizontally) is tied to
/// the post instead, and the Lead is used. `None` when `post` isn't a fence
/// post, the hand holds no Lead, or no mob of the actor's is near enough.
/// Single-player's right-click and a joiner's `LeadToPost` (review D2b B3).
pub fn lead_to_post(
    ecs: &mut hecs::World,
    world: &crate::world::World,
    post: [i32; 3],
    held: Option<&Item>,
    actor: &Actor,
) -> Option<Interaction> {
    if held_material(held) != Some(MaterialId::Lead)
        || !crate::block::is_fence_post(world.get_block(post[0], post[1], post[2]))
    {
        return None;
    }
    let anchor = glam::Vec3::new(post[0] as f32 + 0.5, post[1] as f32 + 1.0, post[2] as f32 + 0.5);
    let mut best: Option<(hecs::Entity, f32)> = None;
    for (id, (pos, tether)) in ecs.query::<(&Position, &Tethered)>().iter() {
        if tether.target != actor.tether {
            continue;
        }
        let dist = glam::Vec2::new(pos.0.x - anchor.x, pos.0.z - anchor.z).length();
        if dist <= POST_TRANSFER_RADIUS && best.is_none_or(|(_, d)| dist < d) {
            best = Some((id, dist));
        }
    }
    let (mob, _) = best?;
    let _ = ecs.insert_one(mob, Tethered { target: TetherTarget::Post(post) });
    Some(Interaction::done(1, InteractNote::None))
}

/// How far (horizontally, in blocks) from a fence post a leashed mob may be
/// for a Lead on the post to tie it there ([`lead_to_post`]).
pub const POST_TRANSFER_RADIUS: f32 = 4.0;

/// Does a joiner's interaction `action` need animal life the server may not
/// simulate (`GameServer::animal_life_simulated`, review D2b MEDIUM-2)?
/// Breeding food needs the breeding step; a tame needs the pet to follow
/// (and a wolf's or Nostrich's auto-leash, the Leads); a Lead on a mob or a
/// post needs the Leads; a pet command needs the pets' AI. Shearing, milking
/// and taking a Lead off need nothing that runs over time.
pub fn needs_animal_life(action: InteractKind) -> bool {
    match action {
        InteractKind::Feed
        | InteractKind::Tame
        | InteractKind::LeadAttach
        | InteractKind::SitToggle
        | InteractKind::LeadToPost { .. } => true,
        InteractKind::Shear | InteractKind::Milk | InteractKind::LeadDetach => false,
    }
}

/// A joiner's `EntityInteract` of kind `action` on the mob `target`, by the
/// same rules (`None` = it does not apply: refused). `LeadToPost` names no
/// mob and is [`lead_to_post`]'s.
pub fn run(
    ecs: &mut hecs::World,
    target: hecs::Entity,
    kind: MobType,
    action: InteractKind,
    held: Option<&Item>,
    sneak: bool,
    actor: &Actor,
    tick: u64,
) -> Option<Interaction> {
    match action {
        InteractKind::Feed => feed(ecs, target, kind, held, sneak, actor.who, tick),
        InteractKind::Tame => tame(ecs, target, kind, held, actor, tick),
        InteractKind::Shear => shear(ecs, target, kind, held, tick),
        InteractKind::Milk => milk(ecs, target, kind, held, tick),
        InteractKind::LeadAttach => lead_attach(ecs, target, kind, held, actor),
        InteractKind::LeadDetach => lead_detach(ecs, target, held),
        InteractKind::SitToggle => sit_toggle(ecs, target, kind, held, actor),
        InteractKind::LeadToPost { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn actor(key: Option<&str>) -> Actor<'_> {
        Actor {
            owner_key: key,
            tether: TetherTarget::Player(0),
            who: crate::combat::Attacker::Local(0),
        }
    }

    fn mat(m: MaterialId) -> Item {
        Item::Material(m)
    }

    #[test]
    fn notes_round_trip_the_wire_and_unknown_codes_read_as_nothing() {
        for code in 0..=17u8 {
            assert_eq!(InteractNote::from_wire(code).to_wire(), code);
        }
        assert_eq!(InteractNote::from_wire(200), InteractNote::None);
    }

    #[test]
    fn a_guest_cannot_tame_and_keeps_the_food() {
        let mut ecs = hecs::World::new();
        let wolf = crate::entity::spawn_mob(&mut ecs, MobType::Wolf, Vec3::new(0.0, 64.0, 0.0));
        let bone = mat(MaterialId::Bone);
        let r = tame(&mut ecs, wolf, MobType::Wolf, Some(&bone), &actor(None), 7).unwrap();
        assert!(!r.done);
        assert_eq!(r.consume, 0);
        assert_eq!(r.note, InteractNote::SignInToTame);
        assert!(crate::tameable::pet_owner_of(&ecs, wolf).is_none());
    }

    #[test]
    fn sit_toggle_is_the_owners_alone() {
        let mut ecs = hecs::World::new();
        let wolf = crate::entity::spawn_mob(&mut ecs, MobType::Wolf, Vec3::new(0.0, 64.0, 0.0));
        ecs.get::<&mut crate::wolf::WolfData>(wolf).unwrap().ownership.owner_pubkey =
            "npub1owner".into();
        let r = sit_toggle(&mut ecs, wolf, MobType::Wolf, None, &actor(Some("npub1other"))).unwrap();
        assert!(!r.done);
        assert_eq!(r.note, InteractNote::NotYourPet);
        let r = sit_toggle(&mut ecs, wolf, MobType::Wolf, None, &actor(Some("npub1owner"))).unwrap();
        assert!(r.done);
        assert_eq!(r.note, InteractNote::Sat);
    }

    /// Review D2b B3 — the fence-post transfer (single-player's Path B and
    /// a joiner's `LeadToPost`): the actor's OWN leashed mob nearest the post
    /// within 4 blocks is tied to it, a Lead used; someone else's leashed mob
    /// and a mob too far off are left alone; no post, no Lead: nothing.
    #[test]
    fn a_lead_on_a_fence_post_ties_the_actors_nearest_leashed_mob() {
        let mut ecs = hecs::World::new();
        let mut world = crate::world::World::new();
        let post = [10, 64, 10];
        let lead = mat(MaterialId::Lead);
        let me = actor(Some("k"));
        let other = Actor { tether: TetherTarget::Player(1), ..me };
        let at = |x: f32| Vec3::new(10.5 + x, 64.0, 10.5);
        let near = crate::entity::spawn_mob(&mut ecs, MobType::Cow, at(1.0));
        let nearer_but_theirs = crate::entity::spawn_mob(&mut ecs, MobType::Cow, at(0.5));
        let far = crate::entity::spawn_mob(&mut ecs, MobType::Cow, at(6.0));
        for (mob, who) in [(near, &me), (nearer_but_theirs, &other), (far, &me)] {
            ecs.insert_one(mob, Tethered { target: who.tether }).unwrap();
        }
        assert!(lead_to_post(&mut ecs, &world, post, Some(&lead), &me).is_none(), "no post there yet");
        world.set_block(post[0], post[1], post[2], crate::block::OAK_FENCE_POST);
        assert!(lead_to_post(&mut ecs, &world, post, None, &me).is_none(), "no Lead in hand");
        let r = lead_to_post(&mut ecs, &world, post, Some(&lead), &me).expect("tied");
        assert!(r.done);
        assert_eq!(r.consume, 1);
        assert_eq!(ecs.get::<&Tethered>(near).unwrap().target, TetherTarget::Post(post));
        assert_eq!(ecs.get::<&Tethered>(nearer_but_theirs).unwrap().target, TetherTarget::Player(1));
        assert_eq!(ecs.get::<&Tethered>(far).unwrap().target, TetherTarget::Player(0));
    }

    #[test]
    fn lead_detach_gives_the_lead_back_and_leaves_an_untethered_mob_alone() {
        let mut ecs = hecs::World::new();
        let cow = crate::entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 0.0));
        assert!(lead_detach(&mut ecs, cow, None).is_none());
        let lead = mat(MaterialId::Lead);
        let on = lead_attach(&mut ecs, cow, MobType::Cow, Some(&lead), &actor(Some("k"))).unwrap();
        assert_eq!(on.consume, 1);
        let off = lead_detach(&mut ecs, cow, None).unwrap();
        assert_eq!(off.give, vec![ItemStack::new_material(MaterialId::Lead, 1)]);
        assert!(ecs.get::<&Tethered>(cow).is_err());
    }
}

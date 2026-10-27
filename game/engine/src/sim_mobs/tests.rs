use super::*;
use crate::mob::MobType;

fn items(ecs: &hecs::World, m: crate::item::MaterialId) -> u32 {
    ecs.query::<&crate::entity::ItemEntity>()
        .iter()
        .filter(|(_, it)| it.stack.item == crate::item::Item::Material(m))
        .map(|(_, it)| u32::from(it.stack.count))
        .sum()
}

/// The product phase lays one chicken egg per cadence (the first at once),
/// however often it runs within one.
#[test]
fn products_lay_one_chicken_egg_per_cadence() {
    let mut ecs = hecs::World::new();
    crate::entity::spawn_mob(&mut ecs, MobType::Chicken, Vec3::new(0.5, 80.0, 0.5));
    let interval = crate::animal_products::EGG_LAY_INTERVAL_TICKS;
    for tick in 0..=2 * interval {
        products(&mut ecs, tick);
    }
    assert_eq!(items(&ecs, crate::item::MaterialId::Egg), 3, "ticks 0, one cadence and two");
}

/// A Lead past its slack pulls the mob toward its holder, over whatever
/// velocity the AI gave it.
#[test]
fn a_lead_pull_overrides_the_mobs_own_velocity() {
    let mut ecs = hecs::World::new();
    let cow = crate::entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(6.5, 80.0, 0.5));
    ecs.insert_one(cow, crate::tether::Tethered { target: crate::tether::TetherTarget::Player(0) }).unwrap();
    ecs.get::<&mut crate::entity::Velocity>(cow).unwrap().0 = Vec3::new(0.2, 0.0, 0.0);
    let bodies = MobBodies { positions: vec![Vec3::new(0.5, 80.0, 0.5)], yaws: vec![0.0], remote_owners: vec![] };
    tethers(&mut ecs, &bodies);
    let v = ecs.get::<&crate::entity::Velocity>(cow).unwrap().0;
    assert!(v.x < 0.0, "pulled toward the holder: {v}");
    assert!(ecs.get::<&crate::tether::Tethered>(cow).is_ok());
}

/// A sneak on a horse gets the rider off before physics moves the mount:
/// the mount is unmounted in place and the seat comes back saying so.
#[test]
fn a_sneak_dismount_unmounts_before_physics_and_says_so() {
    let mut ecs = hecs::World::new();
    let world = World::new();
    let horse = crate::entity::spawn_mob(&mut ecs, MobType::Horse, Vec3::new(0.5, 80.0, 0.5));
    let mut ride = crate::riding::RideState::default();
    crate::riding::mount(&mut ecs, horse, &mut ride);
    let seat = LocalSeat {
        slot: 0,
        mount: horse,
        input: crate::riding::SeatInput { sneak: true, ..Default::default() },
        ride,
        off: None,
    };
    let mut out = SimEvents::default();
    steer_local_seats(&mut ecs, &world, vec![seat], &mut out);
    assert!(ecs.get::<&crate::entity::Ridden>(horse).is_err(), "unmounted");
    match out.take().as_slice() {
        [SimEvent::Seat(s)] => {
            assert_eq!(s.off, Some(crate::riding::Dismount::Sneak));
            assert_eq!(s.ride, crate::riding::RideState::default());
        }
        other => panic!("one seat back, got {other:?}"),
    }
}

/// A forward input steers the mount (its velocity points along the look),
/// and the seat comes back still seated.
#[test]
fn a_forward_input_steers_the_mount() {
    let mut ecs = hecs::World::new();
    let world = World::new();
    let horse = crate::entity::spawn_mob(&mut ecs, MobType::Horse, Vec3::new(0.5, 80.0, 0.5));
    let mut ride = crate::riding::RideState::default();
    crate::riding::mount(&mut ecs, horse, &mut ride);
    let seat = LocalSeat {
        slot: 0,
        mount: horse,
        input: crate::riding::SeatInput { forward: 1.0, yaw: 0.0, ..Default::default() },
        ride,
        off: None,
    };
    let mut out = SimEvents::default();
    steer_local_seats(&mut ecs, &world, vec![seat], &mut out);
    let v = ecs.get::<&crate::entity::Velocity>(horse).unwrap().0;
    assert!(v.z < 0.0, "forward at yaw 0 is -z: {v}");
    assert!(matches!(out.take().as_slice(), [SimEvent::Seat(LocalSeat { off: None, .. })]));
}

/// A baby comes out as one birth event with its credit; one pair, one baby.
#[test]
fn one_fed_pair_makes_one_birth_event() {
    let mut ecs = hecs::World::new();
    for x in [0.5, 1.5] {
        let cow = crate::entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(x, 80.0, 0.5));
        ecs.insert_one(cow, crate::breeding::InLove { until_tick: 1000, fed_by: Some(crate::combat::Attacker::Local(0)) })
            .unwrap();
    }
    let mut out = SimEvents::default();
    breed(&mut ecs, 10, &mut out);
    breed(&mut ecs, 11, &mut out);
    let births: Vec<SimEvent> = out.take();
    assert_eq!(births.len(), 1, "{births:?}");
    let SimEvent::BabyBorn(b) = &births[0] else { panic!("{births:?}") };
    assert!(b.credit_here && b.joiners == [None, None]);
    assert_eq!(b.kind, MobType::Cow);
}

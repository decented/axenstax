use super::*;

#[test]
fn the_window_is_fifteen_seconds_of_server_ticks() {
    assert_eq!(PROTECTION_TICKS, 300, "15 s at 20 ticks a second");
}

#[test]
fn arm_tick_disarm() {
    let mut sp = ServerPlayer::new(glam::Vec3::new(1.5, 70.0, 2.5));
    assert!(!shielded(&sp), "a new body is not protected until it enters the world");
    sp.enter_world();
    assert!(shielded(&sp), "entering the world arms it");
    assert_eq!(targetable_pos(&sp), crate::species_ai::ABSENT_PLAYER);
    for _ in 0..PROTECTION_TICKS - 1 {
        tick(&mut sp);
    }
    assert!(shielded(&sp), "still protected on the last tick");
    tick(&mut sp);
    assert!(!shielded(&sp), "over after {PROTECTION_TICKS} ticks");
    assert_eq!(targetable_pos(&sp), sp.player.pos);
    tick(&mut sp);
    assert_eq!(sp.spawn_protection, 0, "no underflow");
    arm(&mut sp);
    assert!(shielded(&sp));
    disarm(&mut sp);
    assert!(!shielded(&sp));
}

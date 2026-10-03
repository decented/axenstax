//! Per-block metadata byte (Spec 48 — Electricity / Power & Logic).
//!
//! A placed block stores only its `u16` id; directional and stateful blocks
//! (levers, gates, mirrors — and the building-blocks backlog: doors, stairs,
//! slabs) need a small side-band of state. We pack that into one `u8`, stored
//! sparsely in `World::block_meta` (absent ⇒ 0, the plain-block default).
//!
//! Layout (documented per block family at the call sites):
//! ```text
//!   bits 0b0000_0111 — facing (0=Down,1=Up,2=N,3=S,4=W,5=E)
//!   bits 0b0001_1000 — state  (2-bit, device-specific: on/off, op variant, …)
//!   bits 0b1110_0000 — aux    (3-bit, device-specific: gate-op hi bit, fill tier, …)
//! ```
//! Pack/unpack are pure helpers so the layout lives in exactly one place.

pub const FACING_MASK: u8 = 0b0000_0111;
pub const STATE_MASK: u8 = 0b0001_1000;
pub const AUX_MASK: u8 = 0b1110_0000;

/// The six cardinal block faces. Discriminants match the low 3 meta bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[derive(Default)]
pub enum Facing {
    Down = 0,
    #[default]
    Up = 1,
    North = 2,
    South = 3,
    West = 4,
    East = 5,
}


impl Facing {
    /// All six faces, for neighbour scans.
    pub const ALL: [Facing; 6] = [
        Facing::Down,
        Facing::Up,
        Facing::North,
        Facing::South,
        Facing::West,
        Facing::East,
    ];

    /// Decode the low 3 bits. Unused codes (6, 7) clamp to `Down`.
    pub fn from_bits(b: u8) -> Facing {
        match b & FACING_MASK {
            1 => Facing::Up,
            2 => Facing::North,
            3 => Facing::South,
            4 => Facing::West,
            5 => Facing::East,
            _ => Facing::Down,
        }
    }

    pub fn to_bits(self) -> u8 {
        self as u8
    }

    /// Unit step from a block toward this face.
    pub fn offset(self) -> (i32, i32, i32) {
        match self {
            Facing::Down => (0, -1, 0),
            Facing::Up => (0, 1, 0),
            Facing::North => (0, 0, -1),
            Facing::South => (0, 0, 1),
            Facing::West => (-1, 0, 0),
            Facing::East => (1, 0, 0),
        }
    }

    pub fn opposite(self) -> Facing {
        match self {
            Facing::Down => Facing::Up,
            Facing::Up => Facing::Down,
            Facing::North => Facing::South,
            Facing::South => Facing::North,
            Facing::West => Facing::East,
            Facing::East => Facing::West,
        }
    }
}

/// Read the facing field.
pub fn facing(m: u8) -> Facing {
    Facing::from_bits(m)
}
/// Replace the facing field, leaving state + aux untouched.
pub fn with_facing(m: u8, f: Facing) -> u8 {
    (m & !FACING_MASK) | (f.to_bits() & FACING_MASK)
}
/// Read the 2-bit state field (0..=3).
pub fn state(m: u8) -> u8 {
    (m & STATE_MASK) >> 3
}
/// Replace the 2-bit state field, leaving facing + aux untouched.
pub fn with_state(m: u8, s: u8) -> u8 {
    (m & !STATE_MASK) | ((s << 3) & STATE_MASK)
}
/// Read the 3-bit aux field (0..=7).
pub fn aux(m: u8) -> u8 {
    (m & AUX_MASK) >> 5
}
/// Replace the 3-bit aux field, leaving facing + state untouched.
pub fn with_aux(m: u8, a: u8) -> u8 {
    (m & !AUX_MASK) | ((a << 5) & AUX_MASK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facing_bits_round_trip() {
        for f in Facing::ALL {
            assert_eq!(Facing::from_bits(f.to_bits()), f);
        }
    }

    #[test]
    fn facing_unused_codes_clamp_to_down() {
        assert_eq!(Facing::from_bits(6), Facing::Down);
        assert_eq!(Facing::from_bits(7), Facing::Down);
    }

    #[test]
    fn facing_offset_matches_face() {
        assert_eq!(Facing::Up.offset(), (0, 1, 0));
        assert_eq!(Facing::Down.offset(), (0, -1, 0));
        assert_eq!(Facing::North.offset(), (0, 0, -1));
        assert_eq!(Facing::South.offset(), (0, 0, 1));
        assert_eq!(Facing::West.offset(), (-1, 0, 0));
        assert_eq!(Facing::East.offset(), (1, 0, 0));
    }

    #[test]
    fn facing_opposite_pairs() {
        assert_eq!(Facing::Up.opposite(), Facing::Down);
        assert_eq!(Facing::Down.opposite(), Facing::Up);
        assert_eq!(Facing::North.opposite(), Facing::South);
        assert_eq!(Facing::South.opposite(), Facing::North);
        assert_eq!(Facing::West.opposite(), Facing::East);
        assert_eq!(Facing::East.opposite(), Facing::West);
    }

    #[test]
    fn facing_field_pack_unpack() {
        for f in Facing::ALL {
            assert_eq!(facing(with_facing(0, f)), f);
        }
    }

    #[test]
    fn state_field_pack_unpack() {
        for s in 0u8..=3 {
            assert_eq!(state(with_state(0, s)), s);
        }
    }

    #[test]
    fn aux_field_pack_unpack() {
        for a in 0u8..=7 {
            assert_eq!(aux(with_aux(0, a)), a);
        }
    }

    #[test]
    fn fields_are_independent() {
        // Set all three fields, then confirm each reads back unchanged.
        let m = with_aux(with_state(with_facing(0, Facing::East), 3), 5);
        assert_eq!(facing(m), Facing::East);
        assert_eq!(state(m), 3);
        assert_eq!(aux(m), 5);
        // Overwriting facing alone must not disturb state or aux.
        let m2 = with_facing(m, Facing::North);
        assert_eq!(facing(m2), Facing::North);
        assert_eq!(state(m2), 3);
        assert_eq!(aux(m2), 5);
    }
}

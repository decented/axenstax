//! Front-to-back order for the opaque section draws (Spec 03 §5, Perf B2).
//!
//! Drawing the nearest sections first lets the depth test reject the fragments
//! of the sections behind them before they are shaded (less overdraw on weak
//! GPUs). The order is a cache: it is rebuilt only when the camera crosses a
//! section boundary or the set of meshes changes (the caller's `epoch`), never
//! per frame, so the sort costs nothing while the player stands or walks inside
//! a section.

use glam::Vec3;

use crate::chunk::CHUNK_SIZE;

/// A section key, `(cx, cy, cz)`.
pub type SectionKey = (i32, i32, i32);

/// The section a world position is in.
pub fn section_of(p: Vec3) -> SectionKey {
    let cs = CHUNK_SIZE as i32;
    (
        (p.x.floor() as i32).div_euclid(cs),
        (p.y.floor() as i32).div_euclid(cs),
        (p.z.floor() as i32).div_euclid(cs),
    )
}

/// Squared distance, in blocks, from `p` to the centre of section `key`.
fn dist_sq_to_centre(p: Vec3, key: SectionKey) -> f32 {
    let cs = CHUNK_SIZE as f32;
    let centre = Vec3::new(
        key.0 as f32 * cs + cs * 0.5,
        key.1 as f32 * cs + cs * 0.5,
        key.2 as f32 * cs + cs * 0.5,
    );
    (centre - p).length_squared()
}

/// A cached nearest-first ordering of section keys.
#[derive(Debug, Default)]
pub struct SectionOrder {
    /// The camera section and mesh-set epoch the cached `order` was built for.
    built_for: Option<(SectionKey, u64)>,
    order: Vec<SectionKey>,
    /// How many times the order was rebuilt (tests).
    #[cfg(test)]
    rebuilds: u64,
}

impl SectionOrder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Times the order has been rebuilt.
    #[cfg(test)]
    pub fn rebuilds(&self) -> u64 {
        self.rebuilds
    }

    /// The keys nearest the camera first. `keys` is read only when the cache is
    /// stale: when the camera's section differs from last time, or `epoch` (a
    /// counter the owner bumps whenever a key is added or removed) has moved.
    /// Ties are broken by key so the order is deterministic.
    pub fn nearest_first(
        &mut self,
        camera: Vec3,
        epoch: u64,
        keys: impl FnOnce() -> Vec<SectionKey>,
    ) -> &[SectionKey] {
        let stamp = (section_of(camera), epoch);
        if self.built_for != Some(stamp) {
            let mut order = keys();
            // The sort key is computed once per section (not per comparison).
            order.sort_by_cached_key(|&k| (ordered_bits(dist_sq_to_centre(camera, k)), k));
            self.order = order;
            self.built_for = Some(stamp);
            #[cfg(test)]
            {
                self.rebuilds += 1;
            }
        }
        &self.order
    }

    /// The cached order as last built (empty before the first build).
    pub fn current(&self) -> &[SectionKey] {
        &self.order
    }

    /// Forget the cache.
    #[cfg(test)]
    pub fn invalidate(&mut self) {
        self.built_for = None;
    }
}

/// A total-order key for a non-negative finite `f32` (its bit pattern orders
/// the same way for values >= 0).
fn ordered_bits(v: f32) -> u32 {
    if v.is_finite() && v >= 0.0 { v.to_bits() } else { u32::MAX }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<SectionKey> {
        let mut v = Vec::new();
        for x in -3..=3 {
            for z in -3..=3 {
                for y in 0..2 {
                    v.push((x, y, z));
                }
            }
        }
        v
    }

    fn dist(p: Vec3, k: SectionKey) -> f32 {
        dist_sq_to_centre(p, k)
    }

    #[test]
    fn sections_are_ordered_nearest_first() {
        let mut o = SectionOrder::new();
        let cam = Vec3::new(8.0, 24.0, 8.0);
        let order = o.nearest_first(cam, 0, keys).to_vec();
        assert_eq!(order.len(), keys().len());
        for w in order.windows(2) {
            assert!(dist(cam, w[0]) <= dist(cam, w[1]), "{:?} before {:?}", w[0], w[1]);
        }
        assert_eq!(order[0], (0, 1, 0), "the camera's own section leads");
    }

    #[test]
    fn the_order_is_deterministic_and_ties_break_by_key() {
        let cam = Vec3::new(8.0, 8.0, 8.0);
        let mut a = SectionOrder::new();
        let mut b = SectionOrder::new();
        let mut shuffled = keys();
        shuffled.reverse();
        let oa = a.nearest_first(cam, 0, keys).to_vec();
        let ob = b.nearest_first(cam, 0, || shuffled).to_vec();
        assert_eq!(oa, ob);
    }

    #[test]
    fn it_resorts_only_when_the_camera_crosses_a_section_or_the_set_changes() {
        let mut o = SectionOrder::new();
        o.nearest_first(Vec3::new(1.0, 1.0, 1.0), 7, keys);
        assert_eq!(o.rebuilds(), 1);
        // Moving inside the same section: cached (the key source is not even read).
        for p in [Vec3::new(2.0, 3.0, 4.0), Vec3::new(15.9, 15.9, 15.9), Vec3::new(0.0, 0.0, 0.0)] {
            o.nearest_first(p, 7, || panic!("must not re-read the keys"));
        }
        assert_eq!(o.rebuilds(), 1);
        // Crossing a boundary on any axis rebuilds.
        o.nearest_first(Vec3::new(16.0, 1.0, 1.0), 7, keys);
        assert_eq!(o.rebuilds(), 2);
        o.nearest_first(Vec3::new(16.5, 1.0, 1.0), 7, || panic!("same section again"));
        o.nearest_first(Vec3::new(16.5, 16.0, 1.0), 7, keys);
        assert_eq!(o.rebuilds(), 3);
        o.nearest_first(Vec3::new(16.5, 16.5, -0.5), 7, keys);
        assert_eq!(o.rebuilds(), 4, "negative z is the next section down");
        // A changed mesh set rebuilds even though the camera is still.
        o.nearest_first(Vec3::new(16.5, 16.5, -0.5), 8, keys);
        assert_eq!(o.rebuilds(), 5);
        o.nearest_first(Vec3::new(16.5, 16.5, -0.5), 8, || panic!("unchanged"));
        // Invalidate forces the next call to rebuild.
        o.invalidate();
        o.nearest_first(Vec3::new(16.5, 16.5, -0.5), 8, keys);
        assert_eq!(o.rebuilds(), 6);
    }

    #[test]
    fn a_new_mesh_set_is_reflected_after_an_epoch_bump() {
        let mut o = SectionOrder::new();
        let cam = Vec3::new(8.0, 8.0, 8.0);
        let first = o.nearest_first(cam, 0, || vec![(5, 0, 5), (0, 0, 0)]).to_vec();
        assert_eq!(first, vec![(0, 0, 0), (5, 0, 5)]);
        let second = o.nearest_first(cam, 1, || vec![(5, 0, 5), (0, 0, 0), (1, 0, 0)]).to_vec();
        assert_eq!(second, vec![(0, 0, 0), (1, 0, 0), (5, 0, 5)]);
        let none = o.nearest_first(cam, 2, Vec::new).to_vec();
        assert!(none.is_empty());
    }

    #[test]
    fn section_of_floors_negative_coordinates() {
        assert_eq!(section_of(Vec3::new(-0.01, -0.01, -0.01)), (-1, -1, -1));
        assert_eq!(section_of(Vec3::new(15.99, 16.0, -16.0)), (0, 1, -1));
        assert_eq!(section_of(Vec3::new(-16.01, 0.0, 31.9)), (-2, 0, 1));
    }
}

// game/engine/src/screen.rs

//! Screen system — viewport rect + content source for split-screen and future multi-screen.

/// Pixel region within a window surface.
#[derive(Clone, Copy, Debug)]
pub struct ViewportRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl ViewportRect {
    pub fn full(window_width: u32, window_height: u32) -> Self {
        Self { x: 0, y: 0, width: window_width, height: window_height }
    }

    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height.max(1) as f32
    }
}

pub enum ScreenContent {
    /// A local player's first-person view. Index into GameState.players.
    LocalPlayer(usize),
}

pub struct Screen {
    pub viewport: ViewportRect,
    pub content: ScreenContent,
}

pub fn compute_screen_layout(
    num_local_players: usize,
    window_width: u32,
    window_height: u32,
) -> Vec<Screen> {
    match num_local_players {
        1 => vec![Screen {
            viewport: ViewportRect::full(window_width, window_height),
            content: ScreenContent::LocalPlayer(0),
        }],
        2 => {
            let half_w = window_width / 2;
            vec![
                Screen {
                    viewport: ViewportRect { x: 0, y: 0, width: half_w, height: window_height },
                    content: ScreenContent::LocalPlayer(0),
                },
                Screen {
                    viewport: ViewportRect {
                        x: half_w, y: 0,
                        width: window_width - half_w,
                        height: window_height,
                    },
                    content: ScreenContent::LocalPlayer(1),
                },
            ]
        }
        3 | 4 => grid_2x2(num_local_players, window_width, window_height),
        _ => {
            log::warn!("Only 1-4 players supported, got {num_local_players}");
            compute_screen_layout(1, window_width, window_height)
        }
    }
}

/// 2×2 quad-quadrant layout. For n=3, the bottom-right slot is omitted (an
/// empty quadrant the HUD layer can use as a "Press A to join" prompt).
/// Odd-pixel remainders are absorbed by the right column and bottom row, so
/// the quadrants always tile the full window with no missed pixels.
fn grid_2x2(n: usize, w: u32, h: u32) -> Vec<Screen> {
    let half_w = w / 2;
    let half_h = h / 2;
    let right_w = w - half_w;
    let bottom_h = h - half_h;

    let quadrants = [
        (0_usize, 0_u32,   0_u32,   half_w, half_h),
        (1,      half_w,  0,       right_w, half_h),
        (2,      0,       half_h,  half_w,  bottom_h),
        (3,      half_w,  half_h,  right_w, bottom_h),
    ];

    quadrants
        .iter()
        .take(n)
        .map(|&(pidx, x, y, width, height)| Screen {
            viewport: ViewportRect { x, y, width, height },
            content: ScreenContent::LocalPlayer(pidx),
        })
        .collect()
}

/// The fourth quadrant of a 2×2 grid — the slot the 3-player layout leaves
/// empty. The HUD layer uses this to position the "Press A to join" prompt.
pub fn empty_quadrant_rect(w: u32, h: u32) -> ViewportRect {
    let half_w = w / 2;
    let half_h = h / 2;
    ViewportRect {
        x: half_w,
        y: half_h,
        width: w - half_w,
        height: h - half_h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_player_full_screen() {
        let screens = compute_screen_layout(1, 1920, 1080);
        assert_eq!(screens.len(), 1);
        let v = &screens[0].viewport;
        assert_eq!((v.x, v.y, v.width, v.height), (0, 0, 1920, 1080));
    }

    #[test]
    fn two_player_side_by_side() {
        let screens = compute_screen_layout(2, 1920, 1080);
        assert_eq!(screens.len(), 2);
        let v0 = &screens[0].viewport;
        assert_eq!((v0.x, v0.y, v0.width, v0.height), (0, 0, 960, 1080));
        let v1 = &screens[1].viewport;
        assert_eq!((v1.x, v1.y, v1.width, v1.height), (960, 0, 960, 1080));
    }

    #[test]
    fn odd_width_no_gap() {
        let screens = compute_screen_layout(2, 1921, 1080);
        let v0 = &screens[0].viewport;
        let v1 = &screens[1].viewport;
        assert_eq!(v0.width + v1.width, 1921);
        assert_eq!(v1.x, v0.width);
    }

    #[test]
    fn aspect_ratio() {
        let v = ViewportRect { x: 0, y: 0, width: 960, height: 1080 };
        let aspect = v.aspect();
        assert!((aspect - 0.8888889).abs() < 0.001);
    }

    fn rect_tuple(s: &Screen) -> (u32, u32, u32, u32) {
        (s.viewport.x, s.viewport.y, s.viewport.width, s.viewport.height)
    }

    fn player_idx(s: &Screen) -> usize {
        match s.content {
            ScreenContent::LocalPlayer(i) => i,
        }
    }

    #[test]
    fn three_player_2x2_with_empty_bottom_right() {
        let screens = compute_screen_layout(3, 1920, 1080);
        assert_eq!(screens.len(), 3);
        assert_eq!(rect_tuple(&screens[0]), (0,   0,   960, 540));
        assert_eq!(rect_tuple(&screens[1]), (960, 0,   960, 540));
        assert_eq!(rect_tuple(&screens[2]), (0,   540, 960, 540));
        assert_eq!(player_idx(&screens[0]), 0);
        assert_eq!(player_idx(&screens[1]), 1);
        assert_eq!(player_idx(&screens[2]), 2);

        let empty = empty_quadrant_rect(1920, 1080);
        assert_eq!((empty.x, empty.y, empty.width, empty.height), (960, 540, 960, 540));
    }

    #[test]
    fn four_player_2x2() {
        let screens = compute_screen_layout(4, 1920, 1080);
        assert_eq!(screens.len(), 4);
        assert_eq!(rect_tuple(&screens[0]), (0,   0,   960, 540));
        assert_eq!(rect_tuple(&screens[1]), (960, 0,   960, 540));
        assert_eq!(rect_tuple(&screens[2]), (0,   540, 960, 540));
        assert_eq!(rect_tuple(&screens[3]), (960, 540, 960, 540));
        for (i, screen) in screens.iter().enumerate() {
            assert_eq!(player_idx(screen), i);
        }
    }

    #[test]
    fn four_player_odd_dimensions_no_gap() {
        let screens = compute_screen_layout(4, 1921, 1081);
        // Columns must tile the full width on each row.
        assert_eq!(screens[0].viewport.width + screens[1].viewport.width, 1921);
        assert_eq!(screens[2].viewport.width + screens[3].viewport.width, 1921);
        // Rows must tile the full height on each column.
        assert_eq!(screens[0].viewport.height + screens[2].viewport.height, 1081);
        assert_eq!(screens[1].viewport.height + screens[3].viewport.height, 1081);
        // Adjacency: second column starts where the first ends.
        assert_eq!(screens[1].viewport.x, screens[0].viewport.width);
        assert_eq!(screens[3].viewport.x, screens[2].viewport.width);
        // Adjacency: bottom row starts where the top row ends.
        assert_eq!(screens[2].viewport.y, screens[0].viewport.height);
        assert_eq!(screens[3].viewport.y, screens[1].viewport.height);
    }

    #[test]
    fn four_player_aspect_ratio() {
        let screens = compute_screen_layout(4, 1920, 1080);
        for screen in &screens {
            let aspect = screen.viewport.aspect();
            assert!(aspect.is_finite());
            // Each quadrant is 960×540 → aspect ≈ 1.7778 (matches the parent 16:9).
            assert!((aspect - 1.7777778).abs() < 0.001);
        }
    }

    #[test]
    fn five_player_falls_back_to_single() {
        let screens = compute_screen_layout(5, 1920, 1080);
        assert_eq!(screens.len(), 1);
        assert_eq!(rect_tuple(&screens[0]), (0, 0, 1920, 1080));
        assert_eq!(player_idx(&screens[0]), 0);
    }
}

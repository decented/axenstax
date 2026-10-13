//! Code-native survival silhouettes, scaled inside existing HUD icon bounds.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};

fn point(rect: Rect, x: f32, y: f32) -> Pos2 {
    rect.min + egui::vec2(rect.width() * x, rect.height() * y)
}

/// Two convex halves keep the cleft crisp and preserve the half-heart reading.
pub(crate) fn heart(
    painter: &Painter,
    rect: Rect,
    empty: Color32,
    fill: Option<Color32>,
    half: bool,
) {
    let left: Vec<_> = [
        (0.5, 0.32),
        (0.35, 0.14),
        (0.17, 0.14),
        (0.05, 0.28),
        (0.05, 0.45),
        (0.17, 0.63),
        (0.5, 0.94),
    ]
    .into_iter()
    .map(|(x, y)| point(rect, x, y))
    .collect();
    let right: Vec<_> = left
        .iter()
        .map(|p| Pos2::new(2.0 * rect.center().x - p.x, p.y))
        .collect();
    painter.add(Shape::convex_polygon(
        left.clone(),
        fill.unwrap_or(empty),
        Stroke::NONE,
    ));
    painter.add(Shape::convex_polygon(
        right.clone(),
        if half { empty } else { fill.unwrap_or(empty) },
        Stroke::NONE,
    ));
    let mut outline = left;
    outline.extend(right.into_iter().rev().skip(1).take(5));
    painter.add(Shape::closed_line(
        outline,
        Stroke::new(1.0_f32, Color32::from_rgb(145, 103, 96)),
    ));
}

/// Drumstick with a separate bone; a depleted icon retains its visible outline.
pub(crate) fn drumstick(
    painter: &Painter,
    rect: Rect,
    empty: Color32,
    fill: Option<Color32>,
    half: bool,
) {
    let body: Vec<_> = [
        (0.10, 0.28),
        (0.28, 0.10),
        (0.52, 0.10),
        (0.72, 0.27),
        (0.72, 0.48),
        (0.47, 0.72),
        (0.24, 0.69),
        (0.10, 0.51),
    ]
    .into_iter()
    .map(|(x, y)| point(rect, x, y))
    .collect();
    let bone = if fill.is_some() && !half {
        Color32::from_rgb(235, 222, 194)
    } else {
        Color32::from_rgb(139, 127, 110)
    };
    let a = point(rect, 0.57, 0.57);
    let b = point(rect, 0.81, 0.81);
    let width = rect.width() * 0.17;
    painter.line_segment(
        [a, b],
        Stroke::new(width + 2.0, Color32::from_rgb(25, 29, 26)),
    );
    painter.line_segment([a, b], Stroke::new(width, bone));
    for (x, y) in [(0.85, 0.76), (0.76, 0.85)] {
        painter.circle(
            point(rect, x, y),
            rect.width() * 0.10,
            bone,
            Stroke::new(1.0_f32, Color32::from_rgb(25, 29, 26)),
        );
    }
    painter.add(Shape::convex_polygon(body.clone(), empty, Stroke::NONE));
    if let Some(colour) = fill {
        let clip = if half {
            Rect::from_min_max(rect.min, Pos2::new(rect.center().x, rect.bottom()))
        } else {
            rect
        };
        painter
            .with_clip_rect(painter.clip_rect().intersect(clip))
            .add(Shape::convex_polygon(body.clone(), colour, Stroke::NONE));
    }
    painter.add(Shape::closed_line(
        body,
        Stroke::new(1.0_f32, Color32::from_rgb(158, 129, 90)),
    ));
}

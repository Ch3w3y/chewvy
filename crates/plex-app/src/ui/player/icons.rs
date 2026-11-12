//! Player icons painted as vector shapes: egui's bundled fonts carry none of
//! these glyphs, and shapes stay crisp at every scale factor.

use std::f32::consts::{PI, TAU};

use egui::{pos2, vec2, Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind};

/// Which icon to paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    Back10,
    Forward10,
    Next,
    Volume,
    Muted,
    Subtitles,
    Audio,
    Speed,
    Settings,
    Info,
    Fullscreen,
    ExitFullscreen,
    Chapters,
    ArrowLeft,
    Check,
    Quality,
}

fn stroke(rect: Rect, color: Color32) -> Stroke {
    Stroke::new((rect.width() / 12.0).max(1.5), color)
}

fn arc(center: Pos2, radius: f32, from: f32, to: f32) -> Vec<Pos2> {
    let steps = 24;
    (0..=steps)
        .map(|i| {
            let a = from + (to - from) * i as f32 / steps as f32;
            center + vec2(a.cos(), a.sin()) * radius
        })
        .collect()
}

/// Paint `icon` inside `rect` in `color`.
pub fn paint(painter: &Painter, icon: Icon, rect: Rect, color: Color32) {
    let r = rect.shrink(rect.width() * 0.12);
    let c = r.center();
    let w = r.width();
    let s = stroke(rect, color);
    match icon {
        Icon::Play => {
            let pts = vec![
                pos2(r.left() + w * 0.22, r.top() + w * 0.1),
                pos2(r.right() - w * 0.08, c.y),
                pos2(r.left() + w * 0.22, r.bottom() - w * 0.1),
            ];
            painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
        }
        Icon::Pause => {
            let bar = vec2(w * 0.22, w * 0.8);
            for x in [c.x - w * 0.2, c.x + w * 0.2] {
                painter.rect_filled(Rect::from_center_size(pos2(x, c.y), bar), 2.0, color);
            }
        }
        Icon::Back10 | Icon::Forward10 => {
            let back = icon == Icon::Back10;
            let radius = w * 0.42;
            let (from, to) = if back {
                (-PI * 0.35, PI * 1.25)
            } else {
                (PI * 1.35, -PI * 0.25)
            };
            let pts = arc(c, radius, from, to);
            if let Some(&tip) = pts.first() {
                let dir = if back { 1.0 } else { -1.0 };
                let head = vec![
                    tip + vec2(-w * 0.16 * dir, -w * 0.02),
                    tip + vec2(w * 0.1 * dir, -w * 0.14),
                    tip + vec2(w * 0.06 * dir, w * 0.12),
                ];
                painter.add(Shape::convex_polygon(head, color, Stroke::NONE));
            }
            painter.add(Shape::line(pts, s));
            painter.text(
                c,
                egui::Align2::CENTER_CENTER,
                "10",
                egui::FontId::proportional(w * 0.34),
                color,
            );
        }
        Icon::Next => {
            let tri = vec![
                pos2(r.left() + w * 0.12, r.top() + w * 0.14),
                pos2(r.right() - w * 0.28, c.y),
                pos2(r.left() + w * 0.12, r.bottom() - w * 0.14),
            ];
            painter.add(Shape::convex_polygon(tri, color, Stroke::NONE));
            let bar =
                Rect::from_center_size(pos2(r.right() - w * 0.16, c.y), vec2(w * 0.12, w * 0.72));
            painter.rect_filled(bar, 1.5, color);
        }
        Icon::Volume | Icon::Muted => {
            let body = vec![
                pos2(r.left() + w * 0.05, c.y - w * 0.15),
                pos2(r.left() + w * 0.25, c.y - w * 0.15),
                pos2(r.left() + w * 0.5, c.y - w * 0.38),
                pos2(r.left() + w * 0.5, c.y + w * 0.38),
                pos2(r.left() + w * 0.25, c.y + w * 0.15),
                pos2(r.left() + w * 0.05, c.y + w * 0.15),
            ];
            painter.add(Shape::convex_polygon(body, color, Stroke::NONE));
            let speaker = pos2(r.left() + w * 0.5, c.y);
            if icon == Icon::Muted {
                let x = pos2(r.right() - w * 0.18, c.y);
                let d = w * 0.16;
                painter.line_segment([x + vec2(-d, -d), x + vec2(d, d)], s);
                painter.line_segment([x + vec2(-d, d), x + vec2(d, -d)], s);
            } else {
                for k in [0.22, 0.4] {
                    painter.add(Shape::line(arc(speaker, w * k, -PI * 0.3, PI * 0.3), s));
                }
            }
        }
        Icon::Subtitles => {
            let frame = Rect::from_center_size(c, vec2(w * 0.95, w * 0.7));
            painter.rect_stroke(frame, 3.0, s, StrokeKind::Inside);
            for (y, x0, x1) in [(0.08, -0.28, 0.0), (0.08, 0.08, 0.28), (0.24, -0.28, 0.12)] {
                painter.line_segment(
                    [
                        pos2(c.x + w * x0, c.y + w * y - w * 0.04),
                        pos2(c.x + w * x1, c.y + w * y - w * 0.04),
                    ],
                    s,
                );
            }
        }
        Icon::Audio => {
            let heights = [0.3, 0.6, 0.85, 0.5, 0.25];
            for (i, h) in heights.iter().enumerate() {
                let x = r.left() + w * (0.1 + 0.2 * i as f32);
                painter.line_segment([pos2(x, c.y - w * h / 2.0), pos2(x, c.y + w * h / 2.0)], s);
            }
        }
        Icon::Speed => {
            painter.add(Shape::line(
                arc(
                    pos2(c.x, c.y + w * 0.12),
                    w * 0.45,
                    PI * 0.95,
                    TAU * 1.0 + PI * 0.05 - PI,
                ),
                s,
            ));
            painter.add(Shape::line(
                arc(pos2(c.x, c.y + w * 0.12), w * 0.45, PI, TAU),
                s,
            ));
            painter.line_segment(
                [
                    pos2(c.x, c.y + w * 0.12),
                    pos2(c.x + w * 0.24, c.y - w * 0.16),
                ],
                s,
            );
            painter.circle_filled(pos2(c.x, c.y + w * 0.12), w * 0.07, color);
        }
        Icon::Settings => {
            let teeth = 8;
            let pts: Vec<Pos2> = (0..teeth * 2)
                .flat_map(|i| {
                    let a0 = i as f32 / (teeth * 2) as f32 * TAU;
                    let a1 = (i as f32 + 1.0) / (teeth * 2) as f32 * TAU;
                    let rad = if i % 2 == 0 { w * 0.48 } else { w * 0.36 };
                    [
                        c + vec2(a0.cos(), a0.sin()) * rad,
                        c + vec2(a1.cos(), a1.sin()) * rad,
                    ]
                })
                .collect();
            painter.add(Shape::closed_line(pts, s));
            painter.circle_stroke(c, w * 0.15, s);
        }
        Icon::Info => {
            painter.circle_stroke(c, w * 0.45, s);
            painter.circle_filled(pos2(c.x, c.y - w * 0.2), w * 0.06, color);
            painter.line_segment([pos2(c.x, c.y - w * 0.04), pos2(c.x, c.y + w * 0.26)], s);
        }
        Icon::Fullscreen | Icon::ExitFullscreen => {
            let k = w * 0.22;
            let out = icon == Icon::Fullscreen;
            for (corner, dx, dy) in [
                (r.left_top(), 1.0, 1.0),
                (r.right_top(), -1.0, 1.0),
                (r.left_bottom(), 1.0, -1.0),
                (r.right_bottom(), -1.0, -1.0),
            ] {
                let p = if out {
                    corner
                } else {
                    corner + vec2(dx * k, dy * k)
                };
                let (sx, sy) = if out { (dx, dy) } else { (-dx, -dy) };
                painter.add(Shape::line(
                    vec![p + vec2(sx * k, 0.0), p, p + vec2(0.0, sy * k)],
                    s,
                ));
            }
        }
        Icon::Chapters => {
            for (i, y) in [-0.28, 0.0, 0.28].into_iter().enumerate() {
                let yy = c.y + w * y;
                painter.circle_filled(pos2(r.left() + w * 0.1, yy), w * 0.06, color);
                let end = if i == 1 { 0.7 } else { 0.9 };
                painter.line_segment(
                    [pos2(r.left() + w * 0.28, yy), pos2(r.left() + w * end, yy)],
                    s,
                );
            }
        }
        Icon::ArrowLeft => {
            painter.add(Shape::line(
                vec![
                    pos2(c.x + w * 0.05, c.y - w * 0.32),
                    pos2(c.x - w * 0.28, c.y),
                    pos2(c.x + w * 0.05, c.y + w * 0.32),
                ],
                s,
            ));
            painter.line_segment([pos2(c.x - w * 0.28, c.y), pos2(c.x + w * 0.38, c.y)], s);
        }
        Icon::Quality => {
            let frame = Rect::from_center_size(c, vec2(w * 1.05, w * 0.72));
            painter.rect_stroke(frame, 3.0, s, StrokeKind::Inside);
            painter.text(
                c,
                egui::Align2::CENTER_CENTER,
                "HD",
                egui::FontId::proportional(w * 0.4),
                color,
            );
        }
        Icon::Check => {
            painter.add(Shape::line(
                vec![
                    pos2(r.left() + w * 0.1, c.y),
                    pos2(c.x - w * 0.1, c.y + w * 0.25),
                    pos2(r.right() - w * 0.05, c.y - w * 0.28),
                ],
                s,
            ));
        }
    }
}

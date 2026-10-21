//! Crisp vector icons painted with egui's native 2D painter.
//! Replaces all emoji glyphs with resolution-independent vector graphics
//! that automatically dynamically adapt between pure white in dark mode
//! and pure black in light mode.

use eframe::egui::{self, pos2, vec2, Color32, Pos2, Rect, Shape, Stroke, StrokeKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Moon,
    Sun,
    Monitor,
    Console,
    Shield,
    Target,
    Gamepad,
    WindowApp,
    Gear,
    Lightning,
    Star,
    Close,
    Checkmark,
    Warning,
    Refresh,
    ChevronDown,
    ChevronRight,
}

/// Resolves the theme color for icons: pure white in dark mode, pure black in light mode.
#[inline]
pub fn theme_icon_color(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::WHITE
    } else {
        Color32::BLACK
    }
}

/// Paints a vector icon into the target `rect` with the specified `color`.
pub fn paint_icon(painter: &egui::Painter, icon: Icon, rect: Rect, color: Color32) {
    let c = rect.center();
    let r = (rect.width().min(rect.height()) * 0.5).max(1.0);

    match icon {
        Icon::Moon => {
            // Crescent moon outline + fill using smooth multi-segment path
            let pts = vec![
                pos2(c.x + r * 0.15, c.y - r * 0.85),
                pos2(c.x - r * 0.35, c.y - r * 0.65),
                pos2(c.x - r * 0.75, c.y - r * 0.25),
                pos2(c.x - r * 0.75, c.y + r * 0.25),
                pos2(c.x - r * 0.35, c.y + r * 0.65),
                pos2(c.x + r * 0.15, c.y + r * 0.85),
                pos2(c.x - r * 0.12, c.y + r * 0.55),
                pos2(c.x - r * 0.28, c.y + r * 0.25),
                pos2(c.x - r * 0.28, c.y - r * 0.25),
                pos2(c.x - r * 0.12, c.y - r * 0.55),
            ];
            painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
        }

        Icon::Sun => {
            // Central sun disc
            painter.circle_filled(c, r * 0.42, color);
            // 8 radiating sunbeams
            for i in 0..8 {
                let a = (i as f32) * std::f32::consts::PI / 4.0;
                let dir = vec2(a.cos(), a.sin());
                let p1 = c + dir * (r * 0.62);
                let p2 = c + dir * (r * 0.95);
                painter.line_segment([p1, p2], Stroke::new(1.4, color));
            }
        }

        Icon::Monitor => {
            // Display frame
            let screen = Rect::from_center_size(pos2(c.x, c.y - r * 0.15), vec2(r * 1.6, r * 1.15));
            painter.rect_stroke(screen, 1.5, Stroke::new(1.4, color), StrokeKind::Inside);
            // Stand neck & base
            painter.line_segment(
                [pos2(c.x, screen.bottom()), pos2(c.x, c.y + r * 0.75)],
                Stroke::new(1.4, color),
            );
            painter.line_segment(
                [pos2(c.x - r * 0.45, c.y + r * 0.75), pos2(c.x + r * 0.45, c.y + r * 0.75)],
                Stroke::new(1.4, color),
            );
        }

        Icon::Console => {
            // Terminal window frame
            let box_rect = Rect::from_center_size(c, vec2(r * 1.65, r * 1.3));
            painter.rect_stroke(box_rect, 2.0, Stroke::new(1.3, color), StrokeKind::Inside);
            // Prompt `>`
            let p1 = pos2(box_rect.min.x + r * 0.3, c.y - r * 0.28);
            let p2 = pos2(box_rect.min.x + r * 0.6, c.y);
            let p3 = pos2(box_rect.min.x + r * 0.3, c.y + r * 0.28);
            painter.line_segment([p1, p2], Stroke::new(1.4, color));
            painter.line_segment([p2, p3], Stroke::new(1.4, color));
            // Cursor `_`
            let u1 = pos2(box_rect.min.x + r * 0.8, c.y + r * 0.28);
            let u2 = pos2(box_rect.min.x + r * 1.25, c.y + r * 0.28);
            painter.line_segment([u1, u2], Stroke::new(1.4, color));
        }

        Icon::Shield => {
            let pts = vec![
                pos2(c.x - r * 0.7, c.y - r * 0.75),
                pos2(c.x, c.y - r * 0.9),
                pos2(c.x + r * 0.7, c.y - r * 0.75),
                pos2(c.x + r * 0.7, c.y + r * 0.05),
                pos2(c.x, c.y + r * 0.9),
                pos2(c.x - r * 0.7, c.y + r * 0.05),
            ];
            for i in 0..pts.len() {
                let next = (i + 1) % pts.len();
                painter.line_segment([pts[i], pts[next]], Stroke::new(1.4, color));
            }
            painter.line_segment(
                [pos2(c.x, c.y - r * 0.85), pos2(c.x, c.y + r * 0.78)],
                Stroke::new(1.2, color),
            );
        }

        Icon::Target => {
            painter.circle_stroke(c, r * 0.72, Stroke::new(1.3, color));
            painter.circle_filled(c, r * 0.2, color);
            painter.line_segment([pos2(c.x, c.y - r * 0.95), pos2(c.x, c.y - r * 0.42)], Stroke::new(1.3, color));
            painter.line_segment([pos2(c.x, c.y + r * 0.42), pos2(c.x, c.y + r * 0.95)], Stroke::new(1.3, color));
            painter.line_segment([pos2(c.x - r * 0.95, c.y), pos2(c.x - r * 0.42, c.y)], Stroke::new(1.3, color));
            painter.line_segment([pos2(c.x + r * 0.42, c.y), pos2(c.x + r * 0.95, c.y)], Stroke::new(1.3, color));
        }

        Icon::Gamepad => {
            let body = Rect::from_center_size(pos2(c.x, c.y + r * 0.05), vec2(r * 1.6, r * 1.0));
            painter.rect_stroke(body, 3.5, Stroke::new(1.4, color), StrokeKind::Inside);
            // D-pad cross on left
            painter.line_segment(
                [pos2(c.x - r * 0.45, c.y - r * 0.22), pos2(c.x - r * 0.45, c.y + r * 0.32)],
                Stroke::new(1.8, color),
            );
            painter.line_segment(
                [pos2(c.x - r * 0.72, c.y + r * 0.05), pos2(c.x - r * 0.18, c.y + r * 0.05)],
                Stroke::new(1.8, color),
            );
            // 4 Face buttons on right
            painter.circle_filled(pos2(c.x + r * 0.45, c.y - r * 0.15), 1.4, color);
            painter.circle_filled(pos2(c.x + r * 0.45, c.y + r * 0.25), 1.4, color);
            painter.circle_filled(pos2(c.x + r * 0.25, c.y + r * 0.05), 1.4, color);
            painter.circle_filled(pos2(c.x + r * 0.65, c.y + r * 0.05), 1.4, color);
        }

        Icon::WindowApp => {
            let win = Rect::from_center_size(c, vec2(r * 1.55, r * 1.25));
            painter.rect_stroke(win, 1.5, Stroke::new(1.3, color), StrokeKind::Inside);
            let bar_y = win.min.y + r * 0.38;
            painter.line_segment([pos2(win.min.x, bar_y), pos2(win.max.x, bar_y)], Stroke::new(1.1, color));
            painter.circle_filled(pos2(win.min.x + r * 0.25, win.min.y + r * 0.2), 1.1, color);
            painter.circle_filled(pos2(win.min.x + r * 0.55, win.min.y + r * 0.2), 1.1, color);
        }

        Icon::Gear => {
            painter.circle_stroke(c, r * 0.52, Stroke::new(1.4, color));
            painter.circle_filled(c, r * 0.22, color);
            for i in 0..6 {
                let a = (i as f32) * std::f32::consts::PI / 3.0;
                let dir = vec2(a.cos(), a.sin());
                let p1 = c + dir * (r * 0.52);
                let p2 = c + dir * (r * 0.88);
                painter.line_segment([p1, p2], Stroke::new(2.4, color));
            }
        }

        Icon::Lightning => {
            let pts = vec![
                pos2(c.x + r * 0.18, c.y - r * 0.9),
                pos2(c.x - r * 0.5, c.y + r * 0.05),
                pos2(c.x - r * 0.05, c.y + r * 0.05),
                pos2(c.x - r * 0.25, c.y + r * 0.9),
                pos2(c.x + r * 0.5, c.y - r * 0.05),
                pos2(c.x + r * 0.05, c.y - r * 0.05),
            ];
            painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
        }

        Icon::Star => {
            let mut pts = Vec::with_capacity(10);
            for i in 0..10 {
                let r_cur = if i % 2 == 0 { r * 0.85 } else { r * 0.38 };
                let a = -std::f32::consts::FRAC_PI_2 + (i as f32) * std::f32::consts::PI / 5.0;
                pts.push(c + vec2(a.cos(), a.sin()) * r_cur);
            }
            // Draw star outline and fill
            for i in 0..pts.len() {
                let next = (i + 1) % pts.len();
                painter.line_segment([pts[i], pts[next]], Stroke::new(1.2, color));
            }
            painter.circle_filled(c, r * 0.38, color);
        }

        Icon::Close => {
            let d = r * 0.55;
            painter.line_segment([pos2(c.x - d, c.y - d), pos2(c.x + d, c.y + d)], Stroke::new(1.7, color));
            painter.line_segment([pos2(c.x - d, c.y + d), pos2(c.x + d, c.y - d)], Stroke::new(1.7, color));
        }

        Icon::Checkmark => {
            let p1 = pos2(c.x - r * 0.65, c.y);
            let p2 = pos2(c.x - r * 0.15, c.y + r * 0.52);
            let p3 = pos2(c.x + r * 0.68, c.y - r * 0.45);
            painter.line_segment([p1, p2], Stroke::new(1.8, color));
            painter.line_segment([p2, p3], Stroke::new(1.8, color));
        }

        Icon::Warning => {
            let p_top = pos2(c.x, c.y - r * 0.85);
            let p_right = pos2(c.x + r * 0.88, c.y + r * 0.75);
            let p_left = pos2(c.x - r * 0.88, c.y + r * 0.75);
            painter.line_segment([p_top, p_right], Stroke::new(1.5, color));
            painter.line_segment([p_right, p_left], Stroke::new(1.5, color));
            painter.line_segment([p_left, p_top], Stroke::new(1.5, color));
            painter.line_segment(
                [pos2(c.x, c.y - r * 0.32), pos2(c.x, c.y + r * 0.18)],
                Stroke::new(1.6, color),
            );
            painter.circle_filled(pos2(c.x, c.y + r * 0.48), 1.2, color);
        }

        Icon::Refresh => {
            let arc_r = r * 0.65;
            let steps = 14;
            let start_a = 0.4f32;
            let end_a = 5.6f32;
            let arc_pts: Vec<Pos2> = (0..=steps)
                .map(|i| {
                    let a = start_a + (end_a - start_a) * (i as f32 / steps as f32);
                    c + vec2(a.cos(), a.sin()) * arc_r
                })
                .collect();
            for w in arc_pts.windows(2) {
                painter.line_segment([w[0], w[1]], Stroke::new(1.4, color));
            }
            if let Some(tip) = arc_pts.last() {
                painter.line_segment([*tip, *tip + vec2(r * 0.25, -r * 0.05)], Stroke::new(1.4, color));
                painter.line_segment([*tip, *tip + vec2(r * 0.05, -r * 0.25)], Stroke::new(1.4, color));
            }
        }

        Icon::ChevronDown => {
            let d = r * 0.45;
            painter.line_segment([pos2(c.x - d, c.y - d * 0.45), pos2(c.x, c.y + d * 0.45)], Stroke::new(1.7, color));
            painter.line_segment([pos2(c.x, c.y + d * 0.45), pos2(c.x + d, c.y - d * 0.45)], Stroke::new(1.7, color));
        }

        Icon::ChevronRight => {
            let d = r * 0.45;
            painter.line_segment([pos2(c.x - d * 0.45, c.y - d), pos2(c.x + d * 0.45, c.y)], Stroke::new(1.7, color));
            painter.line_segment([pos2(c.x + d * 0.45, c.y), pos2(c.x - d * 0.45, c.y + d)], Stroke::new(1.7, color));
        }
    }
}

/// Renders a standalone vector icon with default size (14.0x14.0) using the active theme color.
pub fn ui_icon(ui: &mut egui::Ui, icon: Icon) -> egui::Response {
    ui_icon_size(ui, icon, 14.0, theme_icon_color(ui))
}

/// Renders a standalone vector icon with a specific size and custom color.
pub fn ui_icon_size(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_icon(ui.painter(), icon, rect, color);
    }
    response
}

/// A button widget featuring a vector icon and optional label text.
/// Uses pure white in dark mode and pure black in light mode by default.
pub struct IconButton {
    icon: Icon,
    text: Option<egui::WidgetText>,
    color: Option<Color32>,
    min_size: egui::Vec2,
}

impl IconButton {
    pub fn new(icon: Icon, text: impl Into<egui::WidgetText>) -> Self {
        Self {
            icon,
            text: Some(text.into()),
            color: None,
            min_size: egui::Vec2::ZERO,
        }
    }

    pub fn icon_only(icon: Icon) -> Self {
        Self {
            icon,
            text: None,
            color: None,
            min_size: egui::Vec2::ZERO,
        }
    }

    #[allow(dead_code)]
    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }

    pub fn min_size(mut self, min_size: egui::Vec2) -> Self {
        self.min_size = min_size;
        self
    }
}

impl egui::Widget for IconButton {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let icon_size = 14.0;
        let padding = ui.spacing().button_padding;
        let spacing = 5.0;

        let (desired_size, galley_opt) = if let Some(text) = self.text {
            let font_id = egui::TextStyle::Button.resolve(ui.style());
            let galley = text.into_galley(ui, None, f32::INFINITY, font_id);
            let size = vec2(
                padding.x * 2.0 + icon_size + spacing + galley.size().x,
                padding.y * 2.0 + galley.size().y.max(icon_size),
            )
            .max(self.min_size);
            (size, Some(galley))
        } else {
            let size = vec2(
                icon_size + padding.x * 2.0,
                icon_size + padding.y * 2.0,
            )
            .max(self.min_size);
            (size, None)
        };

        let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let visuals = ui.style().interact(&response);
            ui.painter().rect(
                rect,
                visuals.corner_radius,
                visuals.bg_fill,
                visuals.bg_stroke,
                StrokeKind::Inside,
            );

            let icon_color = if !ui.is_enabled() {
                visuals.text_color()
            } else if let Some(c) = self.color {
                c
            } else {
                theme_icon_color(ui)
            };
            let center_y = rect.center().y;

            if let Some(galley) = galley_opt {
                let icon_rect = Rect::from_center_size(
                    pos2(rect.min.x + padding.x + icon_size * 0.5, center_y),
                    vec2(icon_size, icon_size),
                );
                paint_icon(ui.painter(), self.icon, icon_rect, icon_color);

                let text_pos = pos2(
                    rect.min.x + padding.x + icon_size + spacing,
                    center_y - galley.size().y * 0.5,
                );
                let text_color = self.color.unwrap_or(visuals.text_color());
                ui.painter().galley(text_pos, galley, text_color);
            } else {
                let icon_rect = Rect::from_center_size(rect.center(), vec2(icon_size, icon_size));
                paint_icon(ui.painter(), self.icon, icon_rect, icon_color);
            }
        }
        response
    }
}

/// Convenience function for adding an icon button to the UI.
#[allow(dead_code)]
pub fn icon_btn(ui: &mut egui::Ui, icon: Icon, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add(IconButton::new(icon, text))
}

/// Convenience function for adding an icon-only button to the UI.
#[allow(dead_code)]
pub fn icon_only_btn(ui: &mut egui::Ui, icon: Icon) -> egui::Response {
    ui.add(IconButton::icon_only(icon))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_icon_color() {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(theme_icon_color(ui), Color32::WHITE);
        });
        out.textures_delta.clear();

        ctx.set_visuals(egui::Visuals::light());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(theme_icon_color(ui), Color32::BLACK);
        });
        out.textures_delta.clear();
    }

    #[test]
    fn test_paint_all_icons_without_panic() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let all_icons = [
                Icon::Moon,
                Icon::Sun,
                Icon::Monitor,
                Icon::Console,
                Icon::Shield,
                Icon::Target,
                Icon::Gamepad,
                Icon::WindowApp,
                Icon::Gear,
                Icon::Lightning,
                Icon::Star,
                Icon::Close,
                Icon::Checkmark,
                Icon::Warning,
                Icon::Refresh,
                Icon::ChevronDown,
                Icon::ChevronRight,
            ];

            let test_rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(16.0, 16.0));
            for icon in all_icons {
                // Paint in white (dark mode)
                paint_icon(ui.painter(), icon, test_rect, Color32::WHITE);
                // Paint in black (light mode)
                paint_icon(ui.painter(), icon, test_rect, Color32::BLACK);
            }
        });
        out.textures_delta.clear();
    }
}

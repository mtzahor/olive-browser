//! The new-tab page: an olive branch, the open-file action, bookmarks as
//! tiles and the most recently visited pages.
use crate::gui::{
    history::{BrowsingHistory, HistoryEntry},
    profile::Bookmark,
    theme::{Palette, Theme},
    ui_icons::{Icon, IconButton},
};
use crate::net::Url;
use eframe::egui::{
    self, Color32, FontId, Pos2, Rect, RichText, Role, Sense, Shape, Stroke, Ui, Vec2, WidgetInfo,
    pos2, text::LayoutJob, vec2,
};

const COLUMN: f32 = 720.0;
const MAX_TILES: usize = 8;
const MAX_RECENT: usize = 4;
const TILE_GAP: f32 = 10.0;
const TILE_HEIGHT: f32 = 60.0;

/// What the person chose on the page.
#[derive(Default)]
pub struct Choice {
    pub open_file: bool,
    pub url: Option<String>,
    pub new_tab: bool,
}

pub fn show(
    ui: &mut Ui,
    theme: Theme,
    bookmarks: &[Bookmark],
    history: &BrowsingHistory,
) -> Choice {
    let palette = theme.palette();
    let mut choice = Choice::default();
    let recent: Vec<_> = history.entries().iter().take(MAX_RECENT).collect();
    let tiles = &bookmarks[..bookmarks.len().min(MAX_TILES)];
    egui::ScrollArea::vertical()
        .id_salt("new-tab-page")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let full = ui.available_rect_before_wrap();
            let column = (full.width() - 32.0).clamp(200.0, COLUMN);
            let rect = Rect::from_min_size(
                pos2(full.center().x - column / 2.0, full.top()),
                vec2(column, full.height()),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                // With nothing saved yet the greeting sits in the middle of the
                // page; otherwise it gives way to bookmarks and recent pages.
                let quiet = tiles.is_empty() && recent.is_empty();
                ui.add_space(if quiet {
                    ((full.height() - 340.0) * 0.4).max(16.0)
                } else {
                    ((full.height() - 660.0) * 0.3).clamp(12.0, 48.0)
                });
                ui.vertical_centered(|ui| {
                    let size = if quiet {
                        vec2(240.0, 104.0)
                    } else {
                        vec2(210.0, 88.0)
                    };
                    let (branch, _) = ui.allocate_exact_size(size, Sense::hover());
                    paint_branch(ui.painter(), branch, theme);
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("A fresh page.")
                            .size(if quiet { 34.0 } else { 30.0 })
                            .color(palette.accent)
                            .variations([("wght", 600.0)]),
                    );
                    ui.add_space(4.0);
                    ui.label(RichText::new("A small browser for the open web.").size(16.0));
                    if quiet {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("Enter a website above, or open a local HTML file.")
                                .size(14.0)
                                .color(palette.muted),
                        );
                    }
                    ui.add_space(if quiet { 20.0 } else { 14.0 });
                    let button = IconButton::new(Icon::Folder, "Open HTML…")
                        .primary()
                        .min_size(vec2(152.0, 40.0));
                    choice.open_file = ui.add(button).clicked();
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(if cfg!(target_os = "macos") {
                            "⌘L to enter an address · ⌘O to open a file"
                        } else {
                            "Ctrl+L to enter an address · Ctrl+O to open a file"
                        })
                        .size(12.0)
                        .color(palette.muted),
                    );
                });
                if !tiles.is_empty() {
                    ui.add_space(28.0);
                    section_heading(ui, palette, Icon::Star, "Bookmarks");
                    bookmark_tiles(ui, palette, tiles, &mut choice);
                }
                if !recent.is_empty() {
                    ui.add_space(if tiles.is_empty() { 28.0 } else { 18.0 });
                    section_heading(ui, palette, Icon::History, "Recently visited");
                    let now = time::OffsetDateTime::now_utc().unix_timestamp();
                    for entry in recent {
                        recent_row(ui, palette, entry, now, &mut choice);
                    }
                }
                ui.add_space(32.0);
            });
        });
    choice
}

fn section_heading(ui: &mut Ui, palette: Palette, icon: Icon, title: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let icon_rect =
        Rect::from_center_size(pos2(rect.left() + 8.0, rect.center().y), vec2(16.0, 16.0));
    icon.paint(ui.painter(), icon_rect, palette.accent);
    let galley = ui.painter().layout_no_wrap(
        title.to_uppercase(),
        FontId::proportional(12.0),
        palette.muted,
    );
    let text_pos = pos2(rect.left() + 24.0, rect.center().y - galley.size().y / 2.0);
    let line_start = text_pos.x + galley.size().x + 12.0;
    ui.painter().galley(text_pos, galley, palette.muted);
    if line_start < rect.right() {
        ui.painter().hline(
            line_start..=rect.right(),
            rect.center().y,
            Stroke::new(1.0, palette.border),
        );
    }
    ui.add_space(6.0);
}

/// Truncated single-line text that fits `width`.
fn text_line(
    ui: &Ui,
    text: &str,
    size: f32,
    color: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), FontId::proportional(size), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(1.0));
    ui.painter().layout_job(job)
}

fn bookmark_tiles(ui: &mut Ui, palette: Palette, tiles: &[Bookmark], choice: &mut Choice) {
    let width = ui.available_width();
    let per_row = (((width + TILE_GAP) / (160.0 + TILE_GAP)).floor() as usize).clamp(1, 4);
    let tile_width = (width - TILE_GAP * (per_row - 1) as f32) / per_row as f32;
    for row in tiles.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = TILE_GAP;
            for bookmark in row {
                let (rect, response) =
                    ui.allocate_exact_size(vec2(tile_width, TILE_HEIGHT), Sense::click());
                let title = if bookmark.title.trim().is_empty() {
                    &bookmark.url
                } else {
                    &bookmark.title
                };
                response.widget_info(|| WidgetInfo::labeled(Role::Link, true, title));
                let response = response.on_hover_text(&bookmark.url);
                let hovered = response.hovered() || response.has_focus();
                ui.painter().rect(
                    rect,
                    12,
                    if hovered {
                        palette.hover
                    } else {
                        palette.surface
                    },
                    Stroke::new(
                        1.0,
                        if hovered {
                            palette.accent
                        } else {
                            palette.border
                        },
                    ),
                    egui::StrokeKind::Inside,
                );
                let host = host_label(&bookmark.url);
                monogram(
                    ui,
                    pos2(rect.left() + 28.0, rect.center().y),
                    15.0,
                    &host,
                    title,
                );
                let inner = tile_width - 64.0;
                let title = text_line(ui, title, 14.0, palette.ink, inner);
                ui.painter().galley(
                    pos2(rect.left() + 52.0, rect.top() + 11.0),
                    title,
                    palette.ink,
                );
                let host = text_line(ui, &host, 12.0, palette.muted, inner);
                ui.painter().galley(
                    pos2(rect.left() + 52.0, rect.top() + 32.0),
                    host,
                    palette.muted,
                );
                record(&response, &bookmark.url, choice);
            }
        });
        ui.add_space(TILE_GAP - ui.spacing().item_spacing.y);
    }
}

fn recent_row(ui: &mut Ui, palette: Palette, entry: &HistoryEntry, now: i64, choice: &mut Choice) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Link, true, entry.display_title()));
    let response = response.on_hover_text(&entry.url);
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 10, palette.hover);
    }
    let remote = entry.url.starts_with("http:") || entry.url.starts_with("https:");
    let icon_rect =
        Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), Vec2::splat(18.0));
    (if remote { Icon::Globe } else { Icon::Page }).paint(ui.painter(), icon_rect, palette.muted);
    let when = text_line(
        ui,
        &relative_time(entry.last_visited, now),
        12.0,
        palette.muted,
        120.0,
    );
    let when_width = when.size().x;
    ui.painter().galley(
        pos2(
            rect.right() - 12.0 - when_width,
            rect.center().y - when.size().y / 2.0,
        ),
        when,
        palette.muted,
    );
    let text_left = rect.left() + 44.0;
    let available = (rect.right() - 24.0 - when_width - text_left).max(40.0);
    let title = text_line(
        ui,
        entry.display_title(),
        14.0,
        palette.ink,
        available * 0.65,
    );
    let title_width = title.size().x;
    let title_height = title.size().y;
    ui.painter().galley(
        pos2(text_left, rect.center().y - title_height / 2.0),
        title,
        palette.ink,
    );
    let host = host_label(&entry.url);
    let host = text_line(
        ui,
        &host,
        12.0,
        palette.muted,
        available - title_width - 10.0,
    );
    ui.painter().galley(
        pos2(
            text_left + title_width + 10.0,
            rect.center().y - host.size().y / 2.0 + 1.0,
        ),
        host,
        palette.muted,
    );
    record(&response, &entry.url, choice);
}

/// Primary click opens here; middle click or Cmd/Ctrl-click opens a new tab.
fn record(response: &egui::Response, url: &str, choice: &mut Choice) {
    let middle = response.clicked_by(egui::PointerButton::Middle);
    if response.clicked() || middle {
        choice.url = Some(url.to_owned());
        choice.new_tab = middle || response.ctx.input(|input| input.modifiers.command);
    }
}

/// The host without a leading `www.`, or the file name of a local page.
pub(crate) fn host_label(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return url.to_owned();
    };
    match parsed.host_str() {
        Some(host) => host.strip_prefix("www.").unwrap_or(host).to_owned(),
        None => parsed
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .filter(|name| !name.is_empty())
            .map(|name| {
                percent_encoding::percent_decode_str(name)
                    .decode_utf8_lossy()
                    .into_owned()
            })
            .unwrap_or_else(|| "Local file".into()),
    }
}

/// A colored disc with the site's initial. Colors come from the grove:
/// leaf, sage, oil, terracotta, bark and ripe olive.
pub(crate) fn monogram(ui: &Ui, center: Pos2, radius: f32, host: &str, title: &str) {
    const GROVE: [Color32; 6] = [
        Color32::from_rgb(92, 113, 55),
        Color32::from_rgb(112, 132, 96),
        Color32::from_rgb(170, 120, 34),
        Color32::from_rgb(176, 98, 58),
        Color32::from_rgb(112, 86, 62),
        Color32::from_rgb(92, 70, 102),
    ];
    let hash = host.bytes().fold(0u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte.into())
    });
    ui.painter()
        .circle_filled(center, radius, GROVE[hash as usize % GROVE.len()]);
    let initial = host
        .chars()
        .chain(title.chars())
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "·".into());
    ui.painter().text(
        center,
        egui::Align2::CENTER_CENTER,
        initial,
        FontId::proportional(radius),
        Color32::WHITE,
    );
}

/// "Just now", "5 min ago", "3 h ago", "Yesterday", "4 days ago", or a date.
fn relative_time(then: i64, now: i64) -> String {
    let seconds = now.saturating_sub(then).max(0);
    match seconds {
        0..60 => "Just now".into(),
        60..3_600 => format!("{} min ago", seconds / 60),
        3_600..86_400 => format!("{} h ago", seconds / 3_600),
        86_400..172_800 => "Yesterday".into(),
        172_800..604_800 => format!("{} days ago", seconds / 86_400),
        _ => time::OffsetDateTime::from_unix_timestamp(then)
            .map(|date| {
                format!(
                    "{} {} {}",
                    date.day(),
                    month_name(date.month()),
                    date.year()
                )
            })
            .unwrap_or_default(),
    }
}

fn month_name(month: time::Month) -> &'static str {
    use time::Month::*;
    match month {
        January => "Jan",
        February => "Feb",
        March => "Mar",
        April => "Apr",
        May => "May",
        June => "Jun",
        July => "Jul",
        August => "Aug",
        September => "Sep",
        October => "Oct",
        November => "Nov",
        December => "Dec",
    }
}

/// An olive sprig: a curved twig, slender leaves with silvery undersides and
/// three olives ripening from green to purple-black.
fn paint_branch(painter: &egui::Painter, rect: Rect, theme: Theme) {
    let (twig, leaf, underside, rib, olives) = match theme {
        Theme::Light => (
            Color32::from_rgb(112, 88, 62),
            Color32::from_rgb(86, 108, 50),
            Color32::from_rgb(140, 160, 112),
            Color32::from_rgba_unmultiplied(236, 242, 222, 150),
            [
                Color32::from_rgb(140, 158, 62),
                Color32::from_rgb(112, 84, 74),
                Color32::from_rgb(70, 52, 78),
            ],
        ),
        Theme::Dark => (
            Color32::from_rgb(158, 132, 100),
            Color32::from_rgb(122, 150, 78),
            Color32::from_rgb(158, 178, 130),
            Color32::from_rgba_unmultiplied(30, 38, 24, 140),
            [
                Color32::from_rgb(162, 180, 80),
                Color32::from_rgb(150, 112, 96),
                Color32::from_rgb(128, 98, 140),
            ],
        ),
    };
    // A gentle quadratic curve from the lower left to the upper right, high
    // enough that the olives and lower leaves stay inside `rect`.
    let from = rect.left_bottom() + vec2(10.0, -rect.height() * 0.45);
    let control = rect.center() + vec2(-10.0, rect.height() * 0.12);
    let to = rect.right_top() + vec2(-24.0, rect.height() * 0.2);
    let at = |t: f32| {
        let u = 1.0 - t;
        pos2(
            u * u * from.x + 2.0 * u * t * control.x + t * t * to.x,
            u * u * from.y + 2.0 * u * t * control.y + t * t * to.y,
        )
    };
    let tangent = |t: f32| {
        let d = 2.0 * (1.0 - t) * (control - from) + 2.0 * t * (to - control);
        d.normalized()
    };
    let twig_points: Vec<_> = (0..=32).map(|i| at(i as f32 / 32.0)).collect();
    painter.add(Shape::line(twig_points, Stroke::new(3.0, twig)));
    painter.circle_filled(from, 1.5, twig);

    let leaf_shape = |base: Pos2, direction: Vec2, length: f32, width: f32, fill: Color32| {
        let normal = direction.rot90();
        let edge: Vec<Pos2> = (0..=12)
            .map(|i| {
                let s = i as f32 / 12.0;
                base + direction * (s * length)
                    + normal * ((s * std::f32::consts::PI).sin() * width / 2.0)
            })
            .chain((1..12).rev().map(|i| {
                let s = i as f32 / 12.0;
                base + direction * (s * length)
                    - normal * ((s * std::f32::consts::PI).sin() * width / 2.0)
            }))
            .collect();
        painter.add(Shape::convex_polygon(edge, fill, Stroke::NONE));
        painter.line_segment(
            [
                base + direction * (length * 0.12),
                base + direction * (length * 0.86),
            ],
            Stroke::new(1.0, rib),
        );
    };
    let rotate = |v: Vec2, degrees: f32| {
        let (sin, cos) = degrees.to_radians().sin_cos();
        vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
    };

    // Olives hang below the twig on short stalks, drawn before the leaves
    // that partly shade them.
    for (index, (t, offset, color)) in [
        (0.34, vec2(-4.0, 20.0), olives[0]),
        (0.5, vec2(6.0, 22.0), olives[2]),
        (0.62, vec2(-2.0, 17.0), olives[1]),
    ]
    .into_iter()
    .enumerate()
    {
        let stalk = at(t);
        let center = stalk + offset;
        painter.line_segment([stalk, center - vec2(0.0, 8.0)], Stroke::new(1.5, twig));
        let tilt = if index % 2 == 0 { 14.0_f32 } else { -10.0 };
        let olive: Vec<Pos2> = (0..24)
            .map(|i| {
                let a = i as f32 / 24.0 * std::f32::consts::TAU;
                center + rotate(vec2(a.cos() * 7.5, a.sin() * 10.0), tilt)
            })
            .collect();
        painter.add(Shape::convex_polygon(olive, color, Stroke::NONE));
        painter.circle_filled(
            center + vec2(-2.5, -3.5),
            2.0,
            Color32::from_white_alpha(70),
        );
    }

    for (index, t) in [0.12_f32, 0.26, 0.4, 0.55, 0.7, 0.84]
        .into_iter()
        .enumerate()
    {
        let side = if index % 2 == 0 { -1.0 } else { 1.0 };
        let direction = rotate(tangent(t), side * 48.0);
        let fill = if index % 3 == 1 { underside } else { leaf };
        let length = 34.0 + 10.0 * (1.0 - (t - 0.5).abs() * 2.0);
        leaf_shape(at(t), direction, length, 9.5, fill);
    }
    leaf_shape(to, tangent(1.0), 34.0, 9.5, leaf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times_read_naturally() {
        let now = 1_760_000_000;
        assert_eq!(relative_time(now - 5, now), "Just now");
        assert_eq!(relative_time(now - 300, now), "5 min ago");
        assert_eq!(relative_time(now - 3 * 3_600, now), "3 h ago");
        assert_eq!(relative_time(now - 30 * 3_600, now), "Yesterday");
        assert_eq!(relative_time(now - 4 * 86_400, now), "4 days ago");
        assert_eq!(relative_time(now - 30 * 86_400, now), "9 Sep 2025");
        // Clock changes must not produce negative ages.
        assert_eq!(relative_time(now + 60, now), "Just now");
    }

    #[test]
    fn host_labels_drop_www_and_name_local_files() {
        assert_eq!(host_label("https://www.example.com/a"), "example.com");
        assert_eq!(host_label("http://localhost:8000/"), "localhost");
        assert_eq!(host_label("file:///tmp/My%20Page.html"), "My Page.html");
        assert_eq!(host_label("not a url"), "not a url");
    }
}

//! Small presentation pieces shared by the screens.
//!
//! Everything is built from the active Omarchy theme's tokens, and every piece
//! is a plain function over borrowed data so the screens stay readable.

use disktree_core::size::{human_bytes, human_bytes_short, share, share_bar};
use disktree_core::space::SpaceInfo;
use disktree_core::tree::{Metric, Node};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Div, ElementId, FontWeight, Hsla, InteractiveElement as _,
    ParentElement, SharedString, Styled, div, relative,
};
use gpui_omarchy::{ActiveTheme, Status};

use crate::ui::{space, text};

use crate::state::Disktree;

/// The scanned root, shortened to `~` where it is the home directory.
pub fn display_root(app: &Disktree) -> String {
    crate::marks::display_path(&app.root_path, app.home.as_deref())
}

/// File counts, which is what a directory count is too.
pub fn human_count(value: u64) -> String {
    disktree_core::size::human_count(value)
}

/// The value to show for a node under the active metric.
pub fn short_value(node: &Node, metric: Metric) -> String {
    match metric {
        Metric::Bytes => human_bytes_short(node.bytes),
        Metric::Files => human_count(node.files),
    }
}

/// The value to show in tables and headers.
pub fn value(node: &Node, metric: Metric) -> String {
    match metric {
        Metric::Bytes => human_bytes(node.bytes),
        Metric::Files => format!("{} files", human_count(node.files)),
    }
}

/// A dim label above a number, for the header strip.
pub fn stat(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    cx: &App,
) -> Div {
    let theme = cx.omarchy();
    div()
        .flex()
        .flex_col()
        .gap(space::XXS)
        .child(
            div()
                .text_size(text::CAPTION)
                .text_color(theme.secondary.opacity(0.75))
                .child(label.into()),
        )
        .child(
            div()
                .text_size(text::TITLE)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.bright)
                .child(value.into()),
        )
}

/// A value with a status colour rather than the neutral one.
pub fn stat_colored(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    color: Hsla,
    cx: &App,
) -> Div {
    let theme = cx.omarchy();
    div()
        .flex()
        .flex_col()
        .gap(space::XXS)
        .child(
            div()
                .text_size(text::CAPTION)
                .text_color(theme.secondary.opacity(0.75))
                .child(label.into()),
        )
        .child(
            div()
                .text_size(text::TITLE)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(value.into()),
        )
}

/// A square chip, used for counts and states.
pub fn chip(label: impl Into<SharedString>, color: Hsla, cx: &App) -> Div {
    let theme = cx.omarchy();
    div()
        .px(space::XS)
        .py(space::XXS)
        .border_1()
        .border_color(color.opacity(0.5))
        .bg(color.opacity(0.1))
        .text_color(color)
        .text_size(text::CAPTION)
        .font_family(theme.font.clone())
        .child(label.into())
}

/// A labelled meter: `label · bar · value`.
pub fn meter_row(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    fraction: f32,
    color: Hsla,
    cx: &App,
) -> Div {
    let theme = cx.omarchy();
    div()
        .flex()
        .flex_col()
        .gap(space::XS)
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .text_size(text::CAPTION)
                .text_color(theme.secondary)
                .child(div().child(label.into()))
                .child(
                    div()
                        .text_color(theme.bright)
                        .font_weight(FontWeight::MEDIUM)
                        .child(value.into()),
                ),
        )
        .child(
            div()
                .w_full()
                .h(space::XS)
                .bg(theme.foreground.opacity(0.08))
                .child(
                    div()
                        .h_full()
                        .w(relative(fraction.clamp(0.0, 1.0)))
                        .bg(color),
                ),
        )
}

/// The volume meter: what is used, what is free, and what the marks will free.
///
/// The projection is drawn as a separate segment so "this much comes back" is
/// visible rather than only stated.
pub fn space_meter(space: SpaceInfo, reclaiming: u64, cx: &App) -> Div {
    let theme = cx.omarchy();
    let total = space.total.max(1);
    let projected = space.after_removing(reclaiming);
    let used_now = space.used() as f32 / total as f32;
    let gained = (projected.used() as f32 / total as f32).max(0.0);
    let used_now = used_now.clamp(0.0, 1.0);
    let gained = gained.clamp(0.0, used_now);

    let label = if reclaiming > 0 {
        format!(
            "{} free · {} after removing {}",
            human_bytes(space.available),
            human_bytes(projected.available),
            human_bytes(reclaiming)
        )
    } else {
        format!(
            "{} free of {}",
            human_bytes(space.available),
            human_bytes(space.total)
        )
    };

    div()
        .flex()
        .flex_col()
        .gap(space::XS)
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .text_size(text::CAPTION)
                .text_color(theme.secondary)
                .child(div().child(label)),
        )
        .child(
            div()
                .relative()
                .w_full()
                .h(space::XS)
                .bg(theme.foreground.opacity(0.08))
                .child(
                    // Used space, as a bar from the left.
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .h_full()
                        .w(relative(used_now))
                        .bg(theme.foreground.opacity(0.22)),
                )
                .child(
                    // What stays used after the removals: the bar shrinks to
                    // here, so the gap is exactly what comes back.
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .h_full()
                        .w(relative(gained))
                        .bg(theme.danger.opacity(0.55)),
                )
                .child(
                    // The reclaimed slice sits at the right edge of what is
                    // currently used.
                    div()
                        .absolute()
                        .left(relative(gained))
                        .top_0()
                        .h_full()
                        .w(relative(used_now - gained))
                        .bg(theme.success.opacity(0.65)),
                ),
        )
}

/// A `key value` hint pair for the bottom bar.
pub fn hint(
    keys: impl Into<SharedString>,
    label: impl Into<SharedString>,
    cx: &App,
) -> Div {
    let theme = cx.omarchy();
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(space::XS)
        .child(gpui_omarchy::keycap(keys, cx))
        .child(
            div()
                .text_size(text::CAPTION)
                .text_color(theme.secondary)
                .child(label.into()),
        )
}

/// A section heading inside a panel.
pub fn section(label: impl Into<SharedString>, cx: &App) -> Div {
    let theme = cx.omarchy();
    div()
        .text_size(text::CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.secondary.opacity(0.85))
        .child(label.into())
}

/// A definition row: label on the left, value on the right.
pub fn row(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    cx: &App,
) -> Div {
    let theme = cx.omarchy();
    div()
        .flex()
        .flex_row()
        .justify_between()
        .gap(space::SM)
        .text_size(text::CAPTION)
        .child(div().text_color(theme.secondary).child(label.into()))
        .child(div().min_w_0().text_color(theme.bright).child(value.into()))
}

/// A block-glyph share bar, which reads as a bar at any font size.
pub fn glyph_bar(
    part: u64,
    total: u64,
    width: usize,
    color: Hsla,
    cx: &App,
) -> Div {
    let theme = cx.omarchy();
    div()
        .text_size(text::CAPTION)
        .font_family(theme.font.clone())
        .text_color(color)
        .child(SharedString::from(share_bar(part, total, width)))
}

/// A percentage with one decimal below ten percent.
pub fn percent(part: u64, total: u64) -> String {
    let value = share(part, total);
    if value < 9.95 {
        format!("{value:.1}%")
    } else {
        format!("{value:.0}%")
    }
}

/// The status colour for an alert.
pub fn alert_color(status: Status, cx: &App) -> Hsla {
    let theme = cx.omarchy();
    match status {
        Status::Neutral => theme.secondary,
        Status::Success => theme.success,
        Status::Warning => theme.warning,
        Status::Error => theme.danger,
    }
}

/// A clickable breadcrumb segment.
pub fn crumb(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    cx: &App,
) -> gpui_kit::Stateful<Div> {
    let theme = cx.omarchy();
    let color = if active {
        theme.bright
    } else {
        theme.secondary
    };
    div()
        .id(id)
        .px(space::XS)
        .py(space::XXS)
        .text_size(text::BODY)
        .text_color(color)
        .when(active, |this| this.font_weight(FontWeight::SEMIBOLD))
        .when(!active, |this| {
            this.hover(|style| style.bg(theme.hover_fill()))
        })
        .child(label.into())
}

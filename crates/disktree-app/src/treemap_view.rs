//! The mosaic: tiles painted on a canvas, labels shaped straight into it.
//!
//! Tiles are painted rather than composed from elements. A treemap can put
//! thousands of rectangles on screen, and an element per rectangle would spend
//! the frame in layout. Painting also means the marked hatch, the selection
//! ring and the hover outline are drawn in one place, in one order.
//!
//! Text is shaped here too. GPUI caches shaped lines, so re-shaping the visible
//! labels every frame costs a lookup, and it lets a label clip exactly to its
//! own tile instead of bleeding into the neighbour.

use std::rc::Rc;

use disktree_core::treemap::Rect;
use gpui_kit::{
    App, Bounds, ContentMask, Context, Corners, Edges, Font, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement as _, Pixels, Point, ScrollWheelEvent,
    SharedString, Size, StatefulInteractiveElement as _, Styled, TextAlign,
    TextRun, Window, canvas, div, pattern_slash, px, quad,
};
use gpui_omarchy::{ActiveTheme, Theme};

use crate::palette;
use crate::state::{Disktree, Label, View};

/// How one tile should be drawn, resolved before the paint callback runs so
/// that painting never has to look anything up.
#[derive(Clone, Debug)]
pub struct TileDeco {
    /// Base-space rectangle: the view transform is applied while painting.
    pub rect: Rect,
    /// The band this directory reserved for its name, when it is subdivided.
    pub header: Option<Rect>,
    pub depth: u32,
    /// Depth from the scanned root, used for the fill so descending does not
    /// recolour the mosaic; `depth` stays view-relative for geometry.
    pub color_depth: u32,
    pub hidden: bool,
    pub marked: bool,
    /// Inside another marked directory, so it goes with its parent.
    pub covered: bool,
    pub hovered: bool,
    pub selected: bool,
}

/// Everything the mosaic needs for one frame.
#[derive(Clone, Debug, Default)]
pub struct Mosaic {
    pub tiles: Vec<TileDeco>,
    pub labels: Vec<Label>,
    pub view: View,
}

/// Build the treemap viewport: canvas, input, and the cursor tooltip.
pub fn mosaic(
    mosaic: Mosaic,
    app: &Disktree,
    window: &Window,
    cx: &Context<'_, Disktree>,
) -> impl IntoElement {
    let theme = cx.omarchy().clone();
    let rem = window.rem_size();
    let origin = Rc::clone(&app.treemap_origin);
    let measured = Rc::clone(&app.treemap_size);

    let colors = Colors::new(&theme);
    let font = theme.font.clone();
    let name_size = rem_px(0.75, rem);
    let size_size = rem_px(0.6875, rem);

    let Mosaic {
        tiles,
        labels,
        view,
    } = mosaic;
    let canvas_origin = Rc::clone(&origin);
    let canvas_measured = Rc::clone(&measured);

    div()
        .id("disktree-treemap")
        .relative()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .overflow_hidden()
        .bg(theme.inset)
        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
            if !hovered {
                this.on_mouse_leave(cx);
            }
        }))
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
            this.on_mouse_move(event, cx);
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.on_mouse_down(event, cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.on_mouse_down(event, cx);
            }),
        )
        .on_scroll_wheel(cx.listener(
            |this, event: &ScrollWheelEvent, _, cx| {
                this.on_scroll_wheel(event, cx);
            },
        ))
        .child(
            canvas(
                move |bounds, window, _| {
                    canvas_origin.set(bounds.origin);
                    // The layout was computed from the previous frame's size. Ask
                    // for one more frame whenever the area is not what we assumed,
                    // which is what makes the first paint and a resize settle.
                    if canvas_measured.get() != bounds.size {
                        canvas_measured.set(bounds.size);
                        window.request_animation_frame();
                    }
                    bounds.size
                },
                move |bounds, _, window, cx| {
                    paint_tiles(&tiles, bounds, view, &colors, window);
                    paint_labels(
                        &labels, bounds, view, &colors, &font, name_size,
                        size_size, window, cx,
                    );
                },
            )
            .absolute()
            .inset_0(),
        )
}

/// Theme colours resolved once per frame.
struct Colors {
    label: Vec<Hsla>,
    label_dim: Hsla,
    border: Hsla,
    hover_border: Hsla,
    selected_border: Hsla,
    marked_border: Hsla,
    marked_hatch: Hsla,
    marked_label: Hsla,
    covered_border: Hsla,
    header_tint: Hsla,
    header_rule: Hsla,
    fill: Vec<Hsla>,
    fill_hidden: Vec<Hsla>,
    fill_marked: Vec<Hsla>,
}

impl Colors {
    fn new(theme: &Theme) -> Self {
        // Absolute depth runs past the visible nesting (each descent adds to
        // it), so the ladder is longer than `max_depth` and clamps at the end.
        let depth_range = 0..=12;
        Self {
            label: depth_range
                .clone()
                .map(|d| palette::label_color(theme, d))
                .collect(),
            label_dim: theme.bright.opacity(0.55),
            border: theme.background.opacity(0.55),
            hover_border: theme.bright.opacity(0.7),
            selected_border: theme.bright,
            marked_border: theme.danger,
            marked_hatch: theme.danger.opacity(0.4),
            marked_label: theme.bright,
            covered_border: theme.danger.opacity(0.45),
            // A band reads as a label strip because it is lifted a little and
            // closed with a rule, not because it is another colour.
            header_tint: theme.bright.opacity(0.09),
            header_rule: theme.background.opacity(0.75),
            fill: depth_range
                .clone()
                .map(|d| palette::tile_fill(theme, d, false, false))
                .collect(),
            fill_hidden: depth_range
                .clone()
                .map(|d| palette::tile_fill(theme, d, true, false))
                .collect(),
            fill_marked: depth_range
                .map(|d| palette::tile_fill(theme, d, false, true))
                .collect(),
        }
    }

    fn fill(&self, tile: &TileDeco) -> Hsla {
        let index = (tile.color_depth as usize).min(self.fill.len() - 1);
        if tile.marked {
            self.fill_marked[index]
        } else if tile.hidden {
            self.fill_hidden[index]
        } else {
            self.fill[index]
        }
    }

    fn label(&self, depth: u32) -> Hsla {
        self.label[(depth as usize).min(self.label.len() - 1)]
    }
}

fn paint_tiles(
    tiles: &[TileDeco],
    bounds: Bounds<Pixels>,
    view: View,
    colors: &Colors,
    window: &mut Window,
) {
    for tile in tiles {
        let rect = view.project(tile.rect);
        if rect.w <= 0.5 || rect.h <= 0.5 {
            continue;
        }
        let quad_bounds = to_window(&rect, bounds);

        // Every tile gets a hairline of background rather than a bright border:
        // that is what separates nested levels without turning the mosaic into
        // a wireframe.
        let mut width = if tile.depth == 0 { 1.0 } else { 0.5 };
        let mut border = colors.border;
        if tile.marked {
            width = 2.0;
            border = colors.marked_border;
        } else if tile.covered {
            width = 1.0;
            border = colors.covered_border;
        }
        if tile.hovered {
            width = 2.0;
            border = colors.hover_border;
        }
        if tile.selected {
            width = 2.0;
            border = colors.selected_border;
        }

        window.paint_quad(quad(
            quad_bounds,
            Corners::default(),
            colors.fill(tile),
            Edges::all(px(width)),
            border,
            gpui_kit::BorderStyle::Solid,
        ));

        // The band a subdivided directory keeps for its name: lifted, and
        // closed with a rule, so the group reads as a group and the space
        // inside it belongs to the children.
        if let Some(header) = tile.header {
            let header = view.project(header);
            if header.w > 1.0 && header.h > 1.0 {
                let band = to_window(&header, bounds);
                window.paint_quad(quad(
                    band,
                    Corners::default(),
                    colors.header_tint,
                    Edges::all(px(0.)),
                    colors.header_rule,
                    gpui_kit::BorderStyle::Solid,
                ));
            }
        }

        // A hatched overlay is the conventional "this one is going away" mark,
        // and it survives on top of any tile colour the theme produces.
        if tile.marked {
            window.paint_quad(quad(
                inset(quad_bounds, px(2.)),
                Corners::default(),
                pattern_slash(colors.marked_hatch, 2.0, 9.0),
                Edges::all(px(0.)),
                colors.marked_border,
                gpui_kit::BorderStyle::Solid,
            ));
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the paint pass threads window state; a struct would only move
              these fields somewhere else"
)]
fn paint_labels(
    labels: &[Label],
    bounds: Bounds<Pixels>,
    view: View,
    colors: &Colors,
    font: &SharedString,
    name_size: Pixels,
    size_size: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    let font = Font {
        family: font.clone(),
        ..Font::default()
    };
    let name_line_height = name_size * 1.35;
    let text_system = window.text_system().clone();
    // Label geometry is proportioned to the label's own type size, which
    // already follows `rem`: padding, thresholds and gaps then keep their
    // relationship to the text at every interface zoom step.
    let text_padding = name_size * 0.42;
    let text_inset = name_size * 0.25;
    let min_width = name_size * 3.3;
    let size_gap = name_size * 0.67;

    for label in labels {
        // A subdivided directory's name lives in the band it reserved; a leaf's
        // sits at the top of its own tile. Either way the mask is the region
        // the label owns, so no label can reach into another tile.
        let owned = label.header.unwrap_or(label.rect);
        let rect = view.project(owned);
        if px(rect.w) < min_width || px(rect.h) < name_size {
            continue;
        }
        let mask = to_window(&rect, bounds);
        let origin = Point::new(
            mask.origin.x + text_padding,
            mask.origin.y + text_inset,
        );
        let color = if label.marked {
            colors.marked_label
        } else {
            colors.label(label.color_depth)
        };

        let run = TextRun {
            len: label.text.len(),
            font: font.clone(),
            color,
            ..TextRun::default()
        };
        let line = text_system.shape_line(
            SharedString::from(label.text.clone()),
            name_size,
            &[run],
            None,
        );

        window.with_content_mask(
            Some(ContentMask { bounds: mask }),
            |window| {
                let _ = line.paint(
                    origin,
                    name_line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );

                if label.size_text.is_empty() {
                    return;
                }
                let room = mask.size.width - (origin.x - mask.origin.x) * 2.;
                let size_run = TextRun {
                    len: label.size_text.len(),
                    font: font.clone(),
                    color: colors.label_dim,
                    ..TextRun::default()
                };
                let size_line = text_system.shape_line(
                    SharedString::from(label.size_text.clone()),
                    size_size,
                    &[size_run],
                    None,
                );
                // Mixed sizes share the name's baseline, not its box. A line
                // paints centred in its line height, putting the baseline at
                // `height / 2 + (ascent - descent) / 2`; equate the two.
                let baseline = origin.y
                    + ((line.ascent - line.descent)
                        - (size_line.ascent - size_line.descent))
                        * 0.5;
                // In a band the size sits at the far end, where it reads as a
                // column; without one it follows the name when there is room.
                let size_origin = if label.header.is_some() {
                    Point::new(
                        mask.origin.x + mask.size.width
                            - size_line.width()
                            - text_padding,
                        baseline,
                    )
                } else {
                    if room - line.width() < size_size * 3.0 {
                        return;
                    }
                    Point::new(origin.x + line.width() + size_gap, baseline)
                };
                if size_origin.x > origin.x + line.width() + text_padding {
                    let _ = size_line.paint(
                        size_origin,
                        name_line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            },
        );
    }
}

fn to_window(rect: &Rect, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    Bounds::new(
        Point::new(bounds.origin.x + px(rect.x), bounds.origin.y + px(rect.y)),
        Size::new(px(rect.w), px(rect.h)),
    )
}

fn inset(bounds: Bounds<Pixels>, amount: Pixels) -> Bounds<Pixels> {
    Bounds::new(
        Point::new(bounds.origin.x + amount, bounds.origin.y + amount),
        Size::new(
            (bounds.size.width - amount * 2.).max(px(0.)),
            (bounds.size.height - amount * 2.).max(px(0.)),
        ),
    )
}

fn rem_px(rems: f32, rem_size: Pixels) -> Pixels {
    px(rems * rem_size.as_f32())
}

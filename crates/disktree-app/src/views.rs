//! The screens: explore, review, running and done.
//!
//! Each function is a pure reading of [`Disktree`], so what a screen shows is
//! always exactly what the state says — there is no second copy of anything to
//! keep in sync.

use disktree_core::removal::{RemovalMode, Target};
use disktree_core::size::human_bytes;
use disktree_core::tree::Metric;
use gpui_kit::{
    App, Context, Div, ElementId, FontWeight, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, Window, div, px,
};
use gpui_omarchy::{
    ActiveTheme, ButtonVariant, ChoiceItem, Theme, alert_dialog, button,
    button_group, dialog_button, dialog_description, dialog_popup,
    dialog_title, separator, toggle, with_tooltip,
};

use gpui_kit::prelude::FluentBuilder as _;

use crate::state::{Disktree, Screen};
use crate::treemap_view::{self, Mosaic};
use crate::ui::{icon, size, space, text};
use crate::widgets;

/// Width, in rem, at which the header holds title, settings and totals on one
/// line. Narrower, the totals move into the status bar.
const HEADER_WIDE_REMS: f32 = 62.0;

/// How many marks the review screen lists. Everything above the cap is still
/// removed; the list only stops being exhaustive, which it says out loud.
const LIST_LIMIT: usize = 1200;

/// The whole window.
pub fn root(
    app: &mut Disktree,
    window: &mut Window,
    cx: &mut Context<'_, Disktree>,
) -> Stateful<Div> {
    let theme = cx.omarchy().clone();
    let body = match app.screen {
        Screen::Explore => explore(app, window, cx),
        Screen::Review => review(app, window, cx),
        Screen::Running => running(app, cx),
        Screen::Done => done(app, cx),
    };

    let mut root = div()
        .id("disktree-root")
        .debug_selector(|| "disktree-root".into())
        .track_focus(&app.focus)
        .key_context("Disktree")
        // The listener is the only place with a window in hand, so it is also
        // where the titlebar is kept in step with the directory on screen.
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            if this.zoom_interface(event, window) {
                cx.notify();
                return;
            }
            let moved = this.on_key_down(event, cx);
            this.apply_focus(window, cx);
            if moved {
                let path = this.current_path();
                let title = format!(
                    "disktree · {}",
                    crate::marks::display_path(&path, this.home.as_deref())
                );
                window.set_window_title(&title);
            }
        }))
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .bg(theme.background)
        .text_color(theme.foreground)
        .font_family(theme.font)
        .text_size(text::BODY)
        .child(body);
    if let Some(tip) = cursor_tooltip(app, window, cx) {
        root = root.child(tip);
    }
    if app.show_help {
        root = root.child(help_overlay(app, cx));
    }
    if app.confirm_open {
        root = root.child(delete_dialog(app, cx));
    }
    root
}

/// The one question disktree asks: a permanent deletion cannot be undone, so
/// it is an alert dialog that names what goes and what comes back. The trash is
/// reversible and needs no dialog.
fn delete_dialog(
    app: &Disktree,
    cx: &mut Context<'_, Disktree>,
) -> impl IntoElement {
    let plan = app.plan();
    let title = match plan.targets.as_slice() {
        [only] => format!(
            "Delete \u{201c}{}\u{201d} permanently?",
            short_name(&only.path)
        ),
        targets => format!("Delete {} items permanently?", targets.len()),
    };
    let body = format!(
        "This frees {}. Deleted files can\u{2019}t be recovered; move them to the trash if you might need them again.",
        human_bytes(plan.bytes())
    );
    let confirm = cx.entity().downgrade();
    let cancel = confirm.clone();
    let actions = div()
        .flex()
        .flex_row()
        .justify_end()
        .gap(space::SM)
        .child(
            dialog_button(
                "delete-cancel",
                "Cancel",
                ButtonVariant::Secondary,
                cx,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.cancel_delete(cx);
                this.apply_focus(window, cx);
            })),
        )
        .child(
            dialog_button(
                "delete-confirm",
                "Delete",
                ButtonVariant::Danger,
                cx,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.confirm_delete(cx);
                this.apply_focus(window, cx);
            })),
        );
    let popup = dialog_popup(cx)
        .child(dialog_title(title, cx))
        .child(dialog_description(body, cx))
        .child(actions);
    alert_dialog(&app.confirm_focus, cx)
        .open(true)
        .on_ok(move |_, window, cx| {
            let _ = confirm.update(cx, |this, cx| {
                this.confirm_delete(cx);
                this.apply_focus(window, cx);
            });
            false
        })
        .on_cancel(move |_, window, cx| {
            let _ = cancel.update(cx, |this, cx| {
                this.cancel_delete(cx);
                this.apply_focus(window, cx);
            });
            false
        })
        .popup(popup)
}

// ── explore ─────────────────────────────────────────────────────────────

fn explore(
    app: &mut Disktree,
    window: &mut Window,
    cx: &mut Context<'_, Disktree>,
) -> Div {
    let theme = cx.omarchy().clone();
    let mosaic: Mosaic = app.prepare();
    let selection = app.show_selection;
    // Below this width the header cannot hold title, settings and totals on
    // one line; the totals move to the status bar rather than wrapping.
    let wide =
        window.viewport_size().width.as_f32() / app.rem >= HEADER_WIDE_REMS;
    let header = explore_header(app, &theme, wide, window, cx);
    let trail = breadcrumbs(app, &theme, cx);

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(header)
        .child(trail)
        .child(div().flex().flex_row().flex_1().min_h_0().child(
            if app.tree().is_none() {
                // The first walk of a home directory takes long enough that
                // an empty viewport would look broken; count the work
                // instead of pretending there is nothing to see.
                scanning_panel(app, &theme, cx).into_any_element()
            } else {
                treemap_view::mosaic(mosaic, app, window, cx).into_any_element()
            },
        ))
        .children(selection.then(|| selection_bar(app, &theme, cx)))
        .child(status_bar(app, &theme, !wide, cx))
        .child(hint_bar(app, &theme, cx))
}

/// What the viewport shows while the first scan is running.
fn scanning_panel(app: &Disktree, theme: &Theme, cx: &gpui_kit::App) -> Div {
    let progress = &app.progress;
    let mut panel = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .gap(space::LG)
        .bg(theme.inset)
        .child(
            gpui_omarchy::icon(gpui_omarchy::IconName::Loader)
                .size(icon::LG)
                .text_color(theme.accent),
        )
        .child(
            div()
                .text_size(text::TITLE)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.bright)
                .child(format!("Reading {}", widgets::display_root(app))),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .gap(space::XL)
                .child(widgets::stat(
                    "files",
                    widgets::human_count(progress.files),
                    cx,
                ))
                .child(widgets::stat("directories", widgets::human_count(progress.dirs), cx))
                .child(widgets::stat("measured", human_bytes(progress.bytes), cx))
                .child(widgets::stat_colored(
                    "unreadable",
                    widgets::human_count(progress.errors),
                    if progress.errors > 0 {
                        theme.warning
                    } else {
                        theme.secondary
                    },
                    cx,
                )),
        )
        .child(
            div()
                .w(size::SCANNING_METER)
                .child(widgets::meter_row(
                    "",
                    "",
                    progress_estimate(progress.files),
                    theme.accent,
                    cx,
                )),
        )
        .child(
            div()
                .text_size(text::BODY)
                .text_color(theme.secondary)
                .child("Marking, zooming and the free-space meter all work as soon as it lands."),
        );

    if let Some(error) = &app.scan_error {
        panel = panel.child(
            div()
                .max_w(size::HELP)
                .p(space::MD)
                .border_1()
                .border_color(theme.danger)
                .text_color(theme.danger)
                .text_size(text::BODY)
                .child(error.clone()),
        );
    }
    panel
}

fn explore_header(
    app: &Disktree,
    theme: &Theme,
    wide: bool,
    window: &mut Window,
    cx: &mut Context<'_, Disktree>,
) -> Div {
    let tree = app.tree();
    let scanned = tree.map_or(0, |node| node.bytes);
    let files = tree.map_or(0, |node| node.files);
    let dirs = tree.map_or(0, |node| node.dirs);

    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(space::MD)
        .px(space::LG)
        .py(space::MD)
        // Leading: the app and what the whole scan found. Trailing: the
        // controls that decide what is measured, on the edge they own. The
        // breadcrumb below names only the place.
        .child(
            div()
                .flex_shrink_0()
                .text_size(text::TITLE)
                .font_weight(FontWeight::BOLD)
                .text_color(theme.bright)
                .child("disktree"),
        );
    if wide {
        row = row
            .child(div().w(space::MD))
            .child(widgets::stat("Scanned", human_bytes(scanned), cx))
            .child(widgets::stat("Files", widgets::human_count(files), cx))
            .child(widgets::stat(
                "Directories",
                widgets::human_count(dirs),
                cx,
            ));
    }
    row = row.child(div().flex_1());
    if app.find_open || !app.find.is_empty() {
        row = row.child(find_field(app, theme));
    }
    row.child(view_settings(app, window, cx))
}

fn find_field(app: &Disktree, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(space::SM)
        .px(space::SM)
        .py(space::XS)
        .border_1()
        .border_color(if app.find_open {
            theme.accent
        } else {
            theme.control_border()
        })
        .bg(theme.normal_fill())
        .text_size(text::BODY)
        .child(
            gpui_omarchy::icon(gpui_omarchy::IconName::Search)
                .size(icon::SM)
                .text_color(theme.secondary),
        )
        .child(if app.find.is_empty() {
            div()
                .text_color(theme.secondary.opacity(0.7))
                .child("Find by name")
        } else {
            div().text_color(theme.bright).child(app.find.clone())
        })
}

fn breadcrumbs(
    app: &Disktree,
    theme: &Theme,
    cx: &Context<'_, Disktree>,
) -> Div {
    let trail = app.breadcrumbs();
    let last = trail.len().saturating_sub(1);
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(space::XXS)
        .px(space::LG)
        .py(space::SM)
        .min_h_0()
        .border_b_1()
        .border_color(theme.divider());

    for (index, (label, crumbs)) in trail.into_iter().enumerate() {
        if index > 0 {
            row = row.child(
                div()
                    .text_color(theme.secondary.opacity(0.6))
                    .text_size(text::CAPTION)
                    .child("/"),
            );
        }
        let active = index == last;
        let id = ElementId::Name(SharedString::from(format!("crumb-{index}")));
        row = row.child(widgets::crumb(id, label, active, cx).on_click(
            cx.listener(move |this, _, _, cx| this.go_to(crumbs.clone(), cx)),
        ));
    }

    row
}

/// How the mosaic is measured and drawn, as real controls with visible state.
///
/// Every control hands the keyboard straight back to the treemap: arrows,
/// Space and Enter belong to the tiles, and a settings click must not quietly
/// capture them.
fn view_settings(
    app: &Disktree,
    window: &mut Window,
    cx: &mut Context<'_, Disktree>,
) -> Div {
    let focus = app.focus.clone();
    let entity = cx.entity().downgrade();

    let ranking = {
        let entity = entity.clone();
        let focus = focus.clone();
        button_group(
            "ranking",
            vec![
                ChoiceItem::new("bytes", "Size"),
                ChoiceItem::new("files", "Files"),
            ],
            Some(usize::from(app.options.metric == Metric::Files)),
            move |index, window, cx| {
                let _ = entity.update(cx, |this, cx| {
                    let wanted = if index == 0 {
                        Metric::Bytes
                    } else {
                        Metric::Files
                    };
                    if this.options.metric != wanted {
                        this.toggle_metric(cx);
                    }
                });
                window.focus(&focus, cx);
            },
            window,
            cx,
        )
        .w(size::RANKING_CHOICE)
        // Tighter than a standalone group, so the segmented control shares
        // the toggles' height and centre line in this toolbar row.
        .p(space::XXS)
    };

    let hidden = {
        let entity = entity.clone();
        let focus = focus.clone();
        toggle("hidden", "Hidden files", app.options.include_hidden, cx)
            .tab_stop(false)
            .on_change(move |_, _, window, cx| {
                let _ = entity.update(cx, |this, cx| {
                    this.options.include_hidden = !this.options.include_hidden;
                    this.start_scan(cx);
                });
                window.focus(&focus, cx);
            })
    };

    let apparent = {
        toggle("apparent", "Apparent size", app.options.apparent_size, cx)
            .tab_stop(false)
            .on_change(move |_, _, window, cx| {
                let _ = entity.update(cx, |this, cx| {
                    this.options.apparent_size = !this.options.apparent_size;
                    this.start_scan(cx);
                });
                window.focus(&focus, cx);
            })
    };

    let depth = app.layout_options.max_depth;
    let levels = with_tooltip(
        button(
            "levels",
            format!("{depth} levels"),
            // Outline, like the toggles beside it: bare text reads as a label.
            ButtonVariant::Outline,
            cx,
        )
        .tab_stop(false)
        .on_click(cx.listener(|this, _, window, cx| {
            // Cycles, so the pointer can reach every depth; the keys step.
            let step = if this.layout_options.max_depth >= 6 {
                -5
            } else {
                1
            };
            this.adjust_depth(step, cx);
            window.focus(&this.focus, cx);
        })),
        "Levels drawn at once \u{00b7} [ and ]",
    );

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(space::SM)
        .child(ranking)
        .child(hidden)
        .child(apparent)
        .child(levels)
}

/// What the keys act on, as one line above the status bar: name and path,
/// size and share, and the two actions that apply to it. The mosaic keeps the
/// full width of the window.
fn selection_bar(
    app: &Disktree,
    theme: &Theme,
    cx: &Context<'_, Disktree>,
) -> Div {
    let target = app.action_target().unwrap_or_else(|| app.crumbs.clone());
    let row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(space::XL)
        .px(space::LG)
        .py(space::SM)
        .border_t_1()
        .border_color(theme.divider())
        .bg(theme.surface);

    let Some(node) = app.node_at(&target) else {
        return row.child(
            div()
                .text_color(theme.secondary)
                .child("Point at a tile or select one with the arrows"),
        );
    };
    let path = app.path_at(&target);
    let parent = target[..target.len().saturating_sub(1)].to_vec();
    let parent_value = app.node_at(&parent).map_or(0, |node| node.bytes);
    let root_value = app.tree().map_or(0, |tree| tree.bytes);
    let marked = path.as_deref().is_some_and(|path| app.marks.contains(path));
    let covered_by = marks_ancestor(app, path.as_deref());
    let is_current_root = target == app.crumbs;

    // Identity: the lane that shrinks, so a long path truncates instead of
    // pushing the actions off the edge.
    let identity = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(space::SM)
        .flex_1()
        .min_w_0()
        .child(
            gpui_omarchy::icon(if node.is_dir() {
                gpui_omarchy::IconName::FolderOpen
            } else {
                gpui_omarchy::IconName::File
            })
            .size(icon::MD)
            .flex_shrink_0()
            .text_color(theme.secondary),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::XXS)
                .min_w_0()
                .overflow_hidden()
                .child(
                    div()
                        .text_size(text::TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.bright)
                        .whitespace_nowrap()
                        .child(node.name.to_string()),
                )
                .child(
                    div()
                        .text_size(text::CAPTION)
                        .text_color(theme.secondary)
                        .whitespace_nowrap()
                        .child(path.as_deref().map_or_else(
                            String::new,
                            |path| {
                                crate::marks::display_path(
                                    path,
                                    app.home.as_deref(),
                                )
                            },
                        )),
                ),
        );

    // Badges stay neutral unless the state is a real deletion or warning.
    let mut badges = div().flex().flex_row().gap(space::XS).flex_shrink_0();
    if node.name.starts_with('.') {
        badges = badges.child(widgets::chip("Hidden", theme.secondary, cx));
    }
    if marked {
        badges = badges.child(widgets::chip("Marked", theme.danger, cx));
    }
    if let Some(ancestor) = &covered_by {
        badges = badges.child(widgets::chip(
            format!("Inside marked {ancestor}"),
            theme.secondary,
            cx,
        ));
    }
    if node.read_error {
        badges = badges.child(widgets::chip("Unreadable", theme.warning, cx));
    }

    // Measure: the size, then its share, then what it holds.
    let measure = div()
        .flex()
        .flex_col()
        .gap(space::XXS)
        .flex_shrink_0()
        .items_end()
        .child(
            div()
                .text_size(text::TITLE)
                .font_weight(FontWeight::BOLD)
                .text_color(theme.bright)
                .child(widgets::value(node, app.options.metric)),
        )
        .child(
            div()
                .text_size(text::CAPTION)
                .text_color(theme.secondary)
                .child(format!(
                    "{} of its directory \u{00b7} {} of the scan \u{00b7} {} files",
                    widgets::percent(node.bytes, parent_value),
                    widgets::percent(node.bytes, root_value),
                    widgets::human_count(node.files),
                )),
        );

    // One primary, only where it is what Enter does. Marking is reversible,
    // so it is framed but neither primary nor danger. Both hand the keyboard
    // back to the treemap.
    let mut actions = div().flex().flex_row().gap(space::SM).flex_shrink_0();
    if !is_current_root && node.is_dir() {
        let crumbs = target.clone();
        actions = actions.child(
            button("open", "Open", ButtonVariant::Primary, cx)
                .tab_stop(false)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.go_to(crumbs.clone(), cx);
                    window.focus(&this.focus, cx);
                })),
        );
    }
    if !is_current_root {
        let crumbs = target;
        actions = actions.child(
            button(
                "mark",
                if marked { "Unmark" } else { "Mark for removal" },
                ButtonVariant::Outline,
                cx,
            )
            .tab_stop(false)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.toggle_mark(&crumbs.clone(), cx);
                window.focus(&this.focus, cx);
            })),
        );
    }

    row.child(identity)
        .child(badges)
        .child(measure)
        .child(actions)
}

fn short_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The marked ancestor of a path, if any, so the UI can explain nesting.
fn marks_ancestor(
    app: &Disktree,
    path: Option<&std::path::Path>,
) -> Option<String> {
    let path = path?;
    app.marks
        .items()
        .iter()
        .filter(|item| item.path != path && path.starts_with(&item.path))
        .map(|item| short_name(&item.path))
        .next()
}

/// The volume and the marks: what was found, what it will free, and the
/// command that acts on the marks, next to the marks it acts on.
fn status_bar(
    app: &Disktree,
    theme: &Theme,
    show_totals: bool,
    cx: &Context<'_, Disktree>,
) -> Div {
    let plan = app.plan();
    let reclaiming = plan.bytes();
    let marked = app.marks.len();
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(space::LG)
        .px(space::LG)
        .py(space::SM)
        .border_t_1()
        .border_color(theme.divider())
        .bg(theme.surface);

    row =
        row.child(div().w(size::SPACE_METER).flex_shrink_0().when_some(
            app.space,
            |this, space| {
                this.child(widgets::space_meter(space, reclaiming, cx))
            },
        ));

    row = row.child(
        div()
            .flex()
            .flex_col()
            .gap(space::XXS)
            .child(widgets::stat_colored(
                "Marked for removal",
                if marked == 0 {
                    "Nothing marked".to_string()
                } else {
                    format!(
                        "{marked} items \u{00b7} {}",
                        human_bytes(reclaiming)
                    )
                },
                if marked > 0 {
                    theme.danger
                } else {
                    theme.secondary
                },
                cx,
            ))
            .child(
                div()
                    .text_size(text::CAPTION)
                    .text_color(theme.secondary)
                    .child(if marked == 0 {
                        "Space marks the tile you point at".to_string()
                    } else if !plan.covered.is_empty()
                        || !plan.blocked.is_empty()
                    {
                        format!(
                            "{} nested \u{00b7} {} kept back",
                            plan.covered.len(),
                            plan.blocked.len()
                        )
                    } else if app.removal_mode == RemovalMode::Trash {
                        "Will move to the trash".to_string()
                    } else {
                        "Will delete permanently".to_string()
                    }),
            ),
    );

    if marked > 0 {
        row = row.child(
            button("review", "Review\u{2026}", ButtonVariant::Secondary, cx)
                .tab_stop(false)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.screen = Screen::Review;
                    cx.notify();
                    window.focus(&this.focus, cx);
                })),
        );
    }

    row = row.child(div().flex_1());

    if app.scan.is_some() {
        let progress = &app.progress;
        row = row.child(
            div()
                .flex()
                .flex_col()
                .gap(space::XS)
                .w(size::SCAN_METER)
                .child(
                    div()
                        .text_size(text::CAPTION)
                        .text_color(theme.secondary)
                        .child(format!(
                            "Scanning \u{00b7} {} files \u{00b7} {}",
                            widgets::human_count(progress.files),
                            human_bytes(progress.bytes)
                        )),
                )
                .child(widgets::meter_row(
                    "",
                    "",
                    progress_estimate(progress.files),
                    theme.accent,
                    cx,
                )),
        );
    } else {
        if show_totals && let Some(tree) = app.tree() {
            row = row.child(widgets::stat(
                "Scanned",
                human_bytes(tree.bytes),
                cx,
            ));
        }
        row = row.child(widgets::stat(
            "Entries read",
            widgets::human_count(app.progress.files),
            cx,
        ));
        if app.progress.errors > 0 {
            row = row.child(widgets::stat_colored(
                "Unreadable",
                format!("{} paths", app.progress.errors),
                theme.warning,
                cx,
            ));
        }
    }

    if let Some((message, status)) = &app.notice {
        row = row.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(space::SM)
                .px(space::SM)
                .py(space::XS)
                .border_1()
                .border_color(widgets::alert_color(*status, cx).opacity(0.5))
                .text_color(widgets::alert_color(*status, cx))
                .text_size(text::CAPTION)
                .child(message.clone()),
        );
    }
    if let Some(error) = &app.scan_error {
        row = row.child(
            div()
                .text_color(theme.danger)
                .text_size(text::CAPTION)
                .child(error.clone()),
        );
    }

    row
}

/// A scan has no total to measure against, so the meter shows the work done
/// rather than pretending to know how far along it is.
fn progress_estimate(files: u64) -> f32 {
    if files == 0 {
        0.06
    } else {
        // Asymptotic: the bar keeps moving while the walk continues.
        let scaled = files as f32 / (files as f32 + 20_000.0);
        (0.1 + scaled * 0.85).min(0.99)
    }
}

fn hint_bar(app: &Disktree, theme: &Theme, cx: &App) -> Div {
    // Most useful first, so a narrow window clips the least useful. The key
    // to every other key is pinned to the trailing edge and never clipped.
    let hints: [(&str, &str); 10] = [
        ("space", "mark"),
        ("enter", "open"),
        ("\u{232b}", "up"),
        ("c", "review"),
        ("\u{2190}\u{2191}\u{2193}\u{2192}", "move"),
        ("/", "find"),
        ("scroll", "zoom"),
        ("[ ]", "levels"),
        ("0", "reset"),
        ("r", "rescan"),
    ];
    let mut lane = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(space::LG)
        .flex_1()
        .min_w_0()
        .overflow_hidden();
    for (keys, label) in hints {
        lane = lane.child(widgets::hint(keys, label, cx).flex_shrink_0());
    }

    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(space::LG)
        .px(space::LG)
        .py(space::SM)
        .border_t_1()
        .border_color(theme.divider())
        .bg(theme.background)
        .child(lane);
    // The magnification only earns space when it is not the default.
    if (app.view.scale - 1.0).abs() > 0.01 {
        row = row.child(
            div()
                .flex_shrink_0()
                .text_size(text::CAPTION)
                .text_color(theme.secondary)
                .child(format!("{:.1}\u{00d7}", app.view.scale)),
        );
    }
    row.child(widgets::hint("?", "all keys", cx).flex_shrink_0())
}

// ── review ──────────────────────────────────────────────────────────────

fn review(
    app: &Disktree,
    window: &mut Window,
    cx: &mut Context<'_, Disktree>,
) -> Div {
    let theme = cx.omarchy().clone();
    let plan = app.plan();
    let items: Vec<Target> = app.marks.items().to_vec();
    let covered: Vec<Target> = plan.covered.clone();
    let blocked = plan.blocked.clone();

    let mut list = div()
        .id("marked-list")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .overflow_y_scroll()
        .border_1()
        .border_color(theme.border)
        .bg(theme.inset);

    if items.is_empty() {
        list = list.child(
            div()
                .p(space::XXL)
                .text_color(theme.secondary)
                .child("Nothing is marked. Go back and mark what should go."),
        );
    }

    for (index, item) in items.iter().take(LIST_LIMIT).enumerate() {
        let is_covered =
            covered.iter().any(|covered| covered.path == item.path);
        let blocked_reason = blocked
            .iter()
            .find(|blocked| blocked.path == item.path)
            .map(|blocked| blocked.reason.clone());
        list = list.child(mark_row(
            index,
            item,
            is_covered,
            blocked_reason,
            app,
            &theme,
            cx,
        ));
    }
    if items.len() > LIST_LIMIT {
        list = list.child(
            div()
                .px(space::MD)
                .py(space::SM)
                .text_size(text::CAPTION)
                .text_color(theme.secondary)
                .child(format!(
                    "{} more are marked and will be removed too. Unmark them in the treemap.",
                    items.len() - LIST_LIMIT
                )),
        );
    }

    let summary = review_summary(app, &plan, &theme, window, cx);
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(screen_header(
            "Review",
            &format!(
                "{} marked \u{00b7} {} to free",
                app.marks.len(),
                human_bytes(plan.bytes())
            ),
            &theme,
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .gap(space::LG)
                .p(space::LG)
                .flex_1()
                .min_h_0()
                .child(list)
                .child(summary),
        )
        .child(review_footer(app, &theme, cx))
}

/// One marked path. Lanes are fixed, so names, bars and sizes line up down
/// the list and sizes can be compared by eye.
fn mark_row(
    index: usize,
    item: &Target,
    covered: bool,
    blocked: Option<String>,
    app: &Disktree,
    theme: &Theme,
    cx: &Context<'_, Disktree>,
) -> Div {
    let root_value = app.tree().map_or(0, |tree| tree.bytes);
    let path_text = crate::marks::display_path(&item.path, app.home.as_deref());
    let color = if blocked.is_some() || covered {
        theme.secondary
    } else {
        theme.foreground
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(space::MD)
        .px(space::MD)
        .py(space::SM)
        .border_b_1()
        .border_color(theme.divider())
        .child(
            gpui_omarchy::icon(if item.is_dir {
                gpui_omarchy::IconName::FolderOpen
            } else {
                gpui_omarchy::IconName::File
            })
            .size(icon::SM)
            .flex_shrink_0()
            .text_color(theme.secondary),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::XXS)
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .min_w_0()
                        .text_color(color)
                        .child(short_name(&item.path)),
                )
                .child(
                    div()
                        .text_size(text::CAPTION)
                        .text_color(theme.secondary)
                        .child(path_text),
                ),
        )
        .children(covered.then(|| {
            widgets::chip("Inside a marked directory", theme.secondary, cx)
        }))
        .children(
            blocked.map(|reason| widgets::chip(reason, theme.warning, cx)),
        )
        .child(div().w(size::SHARE_LANE).flex_shrink_0().child(
            widgets::glyph_bar(
                item.bytes,
                root_value.max(1),
                8,
                theme.secondary,
                cx,
            ),
        ))
        .child(
            // Comparable numbers right-align.
            div()
                .w(size::SIZE_LANE)
                .flex_shrink_0()
                .flex()
                .justify_end()
                .text_color(theme.bright)
                .child(human_bytes(item.bytes)),
        )
        .child(
            button(
                ElementId::Name(SharedString::from(format!("unmark-{index}"))),
                "Unmark",
                ButtonVariant::Outline,
                cx,
            )
            .on_click(cx.listener({
                let path = item.path.clone();
                move |this, _, _, cx| this.unmark(&path.clone(), cx)
            })),
        )
}

fn review_summary(
    app: &Disktree,
    plan: &disktree_core::removal::Plan,
    theme: &Theme,
    window: &mut Window,
    cx: &mut Context<'_, Disktree>,
) -> Div {
    let reclaiming = plan.bytes();
    let trash = app.trash_backend.is_available();

    // One silhouette for one either-or choice, reversible option first.
    let entity = cx.entity().downgrade();
    let mode = button_group(
        "removal-mode",
        vec![
            ChoiceItem::new("trash", "Move to trash").disabled(!trash),
            ChoiceItem::new("permanent", "Delete permanently"),
        ],
        Some(usize::from(app.removal_mode == RemovalMode::Permanent)),
        move |index, _, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.removal_mode = if index == 0 {
                    RemovalMode::Trash
                } else {
                    RemovalMode::Permanent
                };
                cx.notify();
            });
        },
        window,
        cx,
    );

    let explanation = match app.removal_mode {
        RemovalMode::Trash => format!(
            "Recoverable from the trash until it is emptied. Uses {}.",
            app.trash_backend.label()
        ),
        RemovalMode::Permanent => {
            "Deleted at once, like rm -rf. Nothing is recoverable.".to_string()
        }
    };

    let mut panel = div()
        .flex()
        .flex_col()
        .gap(space::XL)
        .w(size::REVIEW_SUMMARY)
        .flex_shrink_0()
        .p(space::LG)
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::SM)
                .child(widgets::section("What happens", cx))
                .child(mode)
                .child(
                    div()
                        .text_size(text::CAPTION)
                        .text_color(theme.secondary)
                        .child(explanation),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::SM)
                .child(widgets::section("Totals", cx))
                .child(widgets::row(
                    "Marked",
                    format!("{}", app.marks.len()),
                    cx,
                ))
                .child(widgets::row(
                    "Acted on",
                    format!("{}", plan.targets.len()),
                    cx,
                ))
                .child(widgets::row(
                    "Nested, go with a parent",
                    format!("{}", plan.covered.len()),
                    cx,
                ))
                .child(widgets::row(
                    "Kept back",
                    format!("{}", plan.blocked.len()),
                    cx,
                ))
                .child(widgets::row(
                    "Space freed",
                    human_bytes(reclaiming),
                    cx,
                )),
        );

    if let Some(volume) = app.space {
        panel = panel.child(
            div()
                .flex()
                .flex_col()
                .gap(space::SM)
                .child(widgets::section("Volume", cx))
                .child(widgets::space_meter(volume, reclaiming, cx)),
        );
    }

    if !plan.blocked.is_empty() {
        let mut blocked = div().flex().flex_col().gap(space::XS);
        for item in plan.blocked.iter().take(6) {
            blocked = blocked.child(
                div()
                    .text_size(text::CAPTION)
                    .text_color(theme.warning)
                    .child(format!(
                        "{}: {}",
                        short_name(&item.path),
                        item.reason
                    )),
            );
        }
        panel = panel.child(
            div()
                .flex()
                .flex_col()
                .gap(space::XS)
                .child(widgets::section("Kept back", cx))
                .child(blocked),
        );
    }

    panel
        .child(div().flex_1())
        .child(commit_controls(app, plan, cx))
}

/// The screen's one commitment. Moving to the trash is the default commit,
/// so it is the primary action; a permanent deletion is destructive, so it is
/// the danger action and asks first, which its ellipsis promises.
fn commit_controls(
    app: &Disktree,
    plan: &disktree_core::removal::Plan,
    cx: &Context<'_, Disktree>,
) -> Div {
    let count = plan.targets.len();
    let noun = if count == 1 { "item" } else { "items" };
    let (label, variant) = match app.removal_mode {
        RemovalMode::Trash => (
            format!("Move {count} {noun} to trash"),
            ButtonVariant::Primary,
        ),
        RemovalMode::Permanent => (
            format!("Delete {count} {noun}\u{2026}"),
            ButtonVariant::Danger,
        ),
    };
    let unavailable = plan.is_empty()
        || (app.removal_mode == RemovalMode::Trash
            && !app.trash_backend.is_available());

    div()
        .flex()
        .flex_row()
        .justify_end()
        .gap(space::SM)
        .child(
            button("back", "Back", ButtonVariant::Secondary, cx).on_click(
                cx.listener(|this, _, window, cx| {
                    this.screen = Screen::Explore;
                    cx.notify();
                    window.focus(&this.focus, cx);
                }),
            ),
        )
        .child(
            button("commit", label, variant, cx)
                .disabled(unavailable)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.commit(cx);
                    this.apply_focus(window, cx);
                })),
        )
}

fn review_footer(app: &Disktree, theme: &Theme, cx: &App) -> Div {
    let commit = match app.removal_mode {
        RemovalMode::Trash => "move to trash",
        RemovalMode::Permanent => "delete\u{2026}",
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(space::LG)
        .px(space::LG)
        .py(space::SM)
        .border_t_1()
        .border_color(theme.divider())
        .child(widgets::hint("enter", commit, cx))
        .child(widgets::hint("m", "trash", cx))
        .child(widgets::hint("p", "permanent", cx))
        .child(widgets::hint("!", "unmark all", cx))
        .child(widgets::hint("esc", "back", cx))
        .child(div().flex_1())
        .child(
            div()
                .text_size(text::CAPTION)
                .text_color(theme.secondary)
                .child(format!(
                    "{} available",
                    app.space.map_or_else(
                        || "?".into(),
                        |space| human_bytes(space.available)
                    )
                )),
        )
}

// ── running ─────────────────────────────────────────────────────────────

fn running(app: &Disktree, cx: &gpui_kit::Context<'_, Disktree>) -> Div {
    let theme = cx.omarchy().clone();
    let summary = app.run_summary.clone();
    let done = summary.removed + summary.failed as u64;
    let progress = if summary.total == 0 {
        0.0
    } else {
        done as f32 / summary.total as f32
    };

    let mut log = div()
        .id("run-log")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .border_1()
        .border_color(theme.border)
        .bg(theme.inset);
    for (path, outcome) in app.run_log.iter().rev().take(200) {
        let ok = outcome.is_ok();
        log = log.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(space::SM)
                .px(space::MD)
                .py(space::XS)
                .text_size(text::CAPTION)
                .child(
                    gpui_omarchy::icon(if ok {
                        gpui_omarchy::IconName::Check
                    } else {
                        gpui_omarchy::IconName::X
                    })
                    .size(icon::SM)
                    .text_color(if ok {
                        theme.success
                    } else {
                        theme.danger
                    }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(if ok {
                            theme.foreground
                        } else {
                            theme.danger
                        })
                        .child(short_name(path)),
                )
                .children(outcome.as_ref().err().map(|error| {
                    div()
                        .text_size(text::CAPTION)
                        .text_color(theme.warning)
                        .child(error.clone())
                })),
        );
    }

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(screen_header(
            "Removing",
            &format!("{} of {} done", done, summary.total),
            &theme,
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::MD)
                .p(space::LG)
                .flex_1()
                .min_h_0()
                .child(widgets::meter_row(
                    "progress",
                    format!(
                        "{} removed · {} to free",
                        summary.removed,
                        human_bytes(summary.bytes)
                    ),
                    progress,
                    if app.removal_mode == RemovalMode::Permanent {
                        theme.danger
                    } else {
                        theme.accent
                    },
                    cx,
                ))
                .children(
                    app.space.map(|space| widgets::space_meter(space, 0, cx)),
                )
                .child(log),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(space::LG)
                .px(space::LG)
                .py(space::SM)
                .border_t_1()
                .border_color(theme.divider())
                .child(widgets::hint("esc", "stop after the current item", cx)),
        )
}

// ── done ────────────────────────────────────────────────────────────────

fn done(app: &Disktree, cx: &gpui_kit::Context<'_, Disktree>) -> Div {
    let theme = cx.omarchy().clone();
    let summary = app.run_summary.clone();
    let measured = match (app.space_baseline, app.space) {
        (Some(before), Some(after)) => Some(
            after
                .available
                .cast_signed()
                .saturating_sub(before.available.cast_signed()),
        ),
        _ => None,
    };

    let mut failures = div().flex().flex_col().gap(space::XXS);
    for (path, outcome) in app
        .run_log
        .iter()
        .filter(|(_, outcome)| outcome.is_err())
        .take(40)
    {
        failures = failures.child(
            div()
                .flex()
                .flex_row()
                .gap(space::SM)
                .text_size(text::CAPTION)
                .child(div().text_color(theme.danger).child(short_name(path)))
                .child(div().text_color(theme.secondary).child(
                    outcome.as_ref().err().cloned().unwrap_or_default(),
                )),
        );
    }

    let mut body = div()
        .flex()
        .flex_col()
        .gap(space::MD)
        .p(space::LG)
        .flex_1()
        .min_h_0()
        .child(
            div()
                .flex()
                .flex_row()
                .gap(space::XXL)
                .child(widgets::stat_colored(
                    "removed",
                    format!("{} items", summary.removed),
                    theme.success,
                    cx,
                ))
                .child(widgets::stat("bytes claimed", human_bytes(summary.bytes), cx))
                .child(widgets::stat_colored(
                    "failed",
                    format!("{}", summary.failed),
                    if summary.failed > 0 {
                        theme.danger
                    } else {
                        theme.secondary
                    },
                    cx,
                ))
                .children(measured.map(|delta| {
                    widgets::stat_colored(
                        "volume freed",
                        if delta >= 0 {
                            format!("+{}", human_bytes(delta.unsigned_abs()))
                        } else {
                            format!("-{}", human_bytes(delta.unsigned_abs()))
                        },
                        if delta >= 0 { theme.success } else { theme.warning },
                        cx,
                    )
                })),
        )
        .child(
            div()
                .text_size(text::CAPTION)
                .text_color(theme.secondary)
                .child("The treemap is being re-scanned so the numbers on screen match the disk again."),
        );

    if let Some(space) = app.space {
        body = body.child(widgets::space_meter(space, 0, cx));
    }
    if app.scan.is_some() {
        body = body.child(widgets::meter_row(
            "re-scanning",
            format!("{} files", widgets::human_count(app.progress.files)),
            progress_estimate(app.progress.files),
            theme.accent,
            cx,
        ));
    }
    if summary.failed > 0 {
        body = body
            .child(separator(cx))
            .child(widgets::section("what could not be removed", cx))
            .child(failures);
    }

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(screen_header("Done", "removal finished", &theme, cx))
        .child(body)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(space::LG)
                .px(space::LG)
                .py(space::SM)
                .border_t_1()
                .border_color(theme.divider())
                .child(
                    // An acknowledgement: the result is already on screen.
                    button("continue", "Done", ButtonVariant::Primary, cx)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.screen = Screen::Explore;
                            cx.notify();
                            window.focus(&this.focus, cx);
                        })),
                )
                .child(widgets::hint("enter", "continue", cx)),
        )
}

// ── shared ──────────────────────────────────────────────────────────────

fn screen_header(
    title: &str,
    subtitle: &str,
    theme: &Theme,
    cx: &gpui_kit::App,
) -> Div {
    let _ = cx;
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(space::MD)
        .px(space::LG)
        .py(space::MD)
        .border_b_1()
        .border_color(theme.divider())
        .child(
            div()
                .text_size(text::TITLE)
                .font_weight(FontWeight::BOLD)
                .text_color(theme.bright)
                .child(title.to_string()),
        )
        .child(
            div()
                .text_size(text::CAPTION)
                .text_color(theme.secondary)
                .child(subtitle.to_string()),
        )
}

/// A tooltip that follows the cursor above everything else in the window.
///
/// It is a layer of the root view rather than a child of the treemap, so it is
/// never clipped at the edge of the mosaic, and it tracks the pointer the
/// hover logic already maintains, so it cannot lag behind the tile it
/// describes.
pub fn cursor_tooltip(
    app: &Disktree,
    window: &Window,
    cx: &gpui_kit::App,
) -> Option<Div> {
    // Placement is in pixels because the pointer is; the card's own size and
    // gaps come from the rem scale so they follow interface zoom.
    const HEIGHT_REMS: f32 = 6.5;
    let rem = window.rem_size().as_f32();
    let width = size::TOOLTIP.0 * rem;
    let height = HEIGHT_REMS * rem;
    let gap = space::MD.0 * rem;
    let edge = space::XS.0 * rem;

    let pointer = app.pointer?;
    let content = hover_tooltip(app, cx)?;
    let origin = app.treemap_origin.get();
    let window_size = window.bounds().size;

    // Flip to the other side of the pointer rather than overflow the window.
    let anchor_x = origin.x.as_f32() + pointer.x.as_f32();
    let anchor_y = origin.y.as_f32() + pointer.y.as_f32();
    let x = if anchor_x + gap + width > window_size.width.as_f32() {
        (anchor_x - gap - width).max(edge)
    } else {
        anchor_x + gap
    };
    let y = if anchor_y + gap + height > window_size.height.as_f32() {
        (anchor_y - gap - height).max(edge)
    } else {
        anchor_y + gap
    };

    // The same surface treatment as Omarchy's tooltip, square and bordered,
    // so an anchored surface of this app does not drift from the system's.
    // Translucent so the mosaic stays visible underneath it.
    let theme = cx.omarchy();
    Some(
        div()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(size::TOOLTIP)
            .flex()
            .flex_col()
            .gap(space::XS)
            .px(space::SM)
            .py(space::SM)
            .border_1()
            .border_color(theme.control_border())
            .bg(theme.background.opacity(0.93))
            .text_color(theme.foreground)
            .font_family(theme.font.clone())
            .text_size(text::CAPTION)
            .child(content),
    )
}

/// The tooltip content for the hovered tile: everything the tile cannot show.
pub fn hover_tooltip(app: &Disktree, cx: &gpui_kit::App) -> Option<Div> {
    let theme = cx.omarchy();
    let crumbs = app.hovered.clone()?;
    let node = app.node_at(&crumbs)?;
    let path = app.path_at(&crumbs);
    let parent = crumbs[..crumbs.len().saturating_sub(1)].to_vec();
    let parent_value = app.node_at(&parent).map_or(0, |node| node.bytes);
    let marked = path.as_deref().is_some_and(|path| app.marks.contains(path));
    let hidden = node.name.starts_with('.');
    let covered = marks_ancestor(app, path.as_deref());

    let mut tip = div()
        .flex()
        .flex_col()
        .gap(space::XS)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(space::XS)
                .child(
                    gpui_omarchy::icon(if node.is_dir() {
                        gpui_omarchy::IconName::FolderOpen
                    } else {
                        gpui_omarchy::IconName::File
                    })
                    .size(icon::SM)
                    .text_color(theme.secondary),
                )
                .child(
                    div()
                        .min_w_0()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.bright)
                        .child(node.name.to_string()),
                ),
        )
        .child(
            div()
                .text_size(text::CAPTION)
                .text_color(theme.secondary)
                .child(path.as_deref().map_or_else(String::new, |path| {
                    crate::marks::display_path(path, app.home.as_deref())
                })),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(space::SM)
                .child(
                    div()
                        .text_size(text::TITLE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.bright)
                        .child(human_bytes(node.bytes)),
                )
                .child(widgets::glyph_bar(
                    node.bytes,
                    parent_value.max(1),
                    10,
                    theme.secondary,
                    cx,
                ))
                .child(
                    div()
                        .text_size(text::CAPTION)
                        .text_color(theme.secondary)
                        .child(widgets::percent(node.bytes, parent_value)),
                ),
        )
        .child(
            div()
                .text_size(text::CAPTION)
                .text_color(theme.secondary)
                .child(format!(
                    "{} files · {} dirs · {} direct",
                    widgets::human_count(node.files),
                    widgets::human_count(
                        node.dirs.saturating_sub(u64::from(node.is_dir()))
                    ),
                    human_bytes(node.own_bytes)
                )),
        );

    let mut badges = div().flex().flex_row().gap(space::XS).flex_wrap();
    if hidden {
        badges = badges.child(widgets::chip("Hidden", theme.secondary, cx));
    }
    if marked {
        badges =
            badges.child(widgets::chip("Marked for removal", theme.danger, cx));
    }
    if let Some(ancestor) = covered {
        badges = badges.child(widgets::chip(
            format!("Inside marked {ancestor}"),
            theme.secondary,
            cx,
        ));
    }
    tip = tip.child(badges);
    tip = tip.child(
        div()
            .text_size(text::CAPTION)
            .text_color(theme.secondary.opacity(0.7))
            .child(if node.is_dir() {
                "space mark · enter open"
            } else {
                "space mark"
            }),
    );
    Some(tip)
}

fn help_overlay(app: &Disktree, cx: &gpui_kit::App) -> Div {
    let theme = cx.omarchy();
    // Sentence case, and the tile a key acts on is always the one under the
    // pointer if the pointer moved last, else the keyboard selection.
    let rows: [(&str, &str); 23] = [
        ("space / x", "Mark or unmark the tile you point at"),
        ("ctrl-click", "Mark without moving the selection"),
        ("enter", "Open that directory, at any depth"),
        ("\u{232b} / esc", "Go up one directory"),
        (
            "\u{2190} \u{2191} \u{2193} \u{2192}",
            "Move between tiles at this level",
        ),
        ("tab", "Next largest sibling"),
        ("scroll", "Zoom toward a directory, then go into it"),
        ("shift-scroll", "Pan the magnified view"),
        ("[ / ]", "Draw fewer or more levels at once"),
        ("- / = / 0", "Magnify, shrink, or reset the view"),
        ("ctrl = / - / 0", "Interface zoom"),
        ("/", "Find an entry by name"),
        ("c", "Review the marked list"),
        ("t", "Rank by size or by file count"),
        ("r", "Scan again from the same root"),
        ("d", "Disk usage or apparent size"),
        ("i", "Include or skip hidden entries"),
        ("p", "Show or hide the selection line"),
        ("q", "Quit"),
        ("", ""),
        (
            "Review screen",
            "m trash \u{00b7} p permanent \u{00b7} ! unmark all",
        ),
        ("", "enter commits \u{00b7} esc goes back"),
        ("", "A permanent deletion always asks first"),
    ];

    let mut keys = div().flex().flex_col().gap(space::SM);
    for (key, label) in rows {
        if key.is_empty() && label.is_empty() {
            continue;
        }
        keys = keys.child(
            div()
                .flex()
                .flex_row()
                .gap(space::MD)
                .items_center()
                .child(
                    div()
                        .w(size::KEY_LANE)
                        .flex_shrink_0()
                        .text_size(text::CAPTION)
                        .text_color(theme.accent)
                        .child(key.to_string()),
                )
                .child(
                    div()
                        .text_size(text::BODY)
                        .text_color(theme.foreground)
                        .child(label.to_string()),
                ),
        );
    }

    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.background.opacity(0.86))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::MD)
                .w(size::HELP)
                .p(space::XL)
                .border_1()
                .border_color(theme.border)
                .bg(theme.surface)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(space::SM)
                        .child(
                            gpui_omarchy::icon(
                                gpui_omarchy::IconName::Keyboard,
                            )
                            .size(icon::MD)
                            .text_color(theme.accent),
                        )
                        .child(
                            div()
                                .text_size(text::TITLE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.bright)
                                .child("Keyboard and mouse"),
                        ),
                )
                .child(keys)
                .child(
                    div()
                        .text_size(text::CAPTION)
                        .text_color(theme.secondary)
                        .child(format!(
                            "? or esc closes \u{00b7} {} \u{00b7} {}",
                            app.root_path.display(),
                            app.options.metric.label()
                        )),
                ),
        )
}

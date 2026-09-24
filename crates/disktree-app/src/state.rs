//! Application state and every mutation the UI can perform.
//!
//! The screens in [`crate::views`] are pure functions of this state; all the
//! decisions — what is selected, what a mark means, what a key does, when to
//! re-scan — live here so they can be reasoned about in one place.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use disktree_core::removal::{
    Plan, RemovalEvent, RemovalHandle, RemovalMode, Target, TrashBackend,
    detect_trash_backend, plan,
};
use disktree_core::scan::{ScanHandle, ScanOptions, ScanSnapshot};
use disktree_core::space::{SpaceInfo, space_info};
use disktree_core::tree::{Node, path_of};
use disktree_core::treemap::{
    LayoutOptions, Rect, Tile, TileKind, hit, layout,
};
use gpui_kit::{
    Context, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, Size,
    Window, px, size,
};
use gpui_omarchy::Status;
use rustc_hash::FxHashSet;

use crate::marks::{Marks, display_path, is_hidden};
use crate::treemap_view::{Mosaic, TileDeco};

/// Which screen the app is showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    /// Walk the treemap and mark what should go.
    #[default]
    Explore,
    /// Review every marked path and choose how to remove them.
    Review,
    /// A removal is running.
    Running,
    /// The removal finished; show what happened.
    Done,
}

/// The treemap view transform: `screen = (base - origin) * scale`.
///
/// All viewport geometry stays in the base space of an unzoomed layout, so pan
/// and zoom never require a re-layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub scale: f32,
    pub origin_x: f32,
    pub origin_y: f32,
}

impl Default for View {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl View {
    pub const IDENTITY: Self = Self {
        scale: 1.0,
        origin_x: 0.0,
        origin_y: 0.0,
    };
    pub const MIN_SCALE: f32 = 1.0;
    /// Past this, zooming again descends into whatever is under the cursor
    /// instead of magnifying further: the continuous "keep zooming and you are
    /// inside" gesture, with a breadcrumb to walk back out.
    pub const MAX_SCALE: f32 = 5.0;

    pub fn project(&self, rect: Rect) -> Rect {
        Rect::new(
            (rect.x - self.origin_x) * self.scale,
            (rect.y - self.origin_y) * self.scale,
            rect.w * self.scale,
            rect.h * self.scale,
        )
    }

    /// Viewport point to base-space point.
    pub fn unproject(&self, x: f32, y: f32) -> (f32, f32) {
        (
            x / self.scale + self.origin_x,
            y / self.scale + self.origin_y,
        )
    }

    /// Zoom by `factor` toward `(x, y)` and no further than `ceiling`.
    ///
    /// The base point under `(x, y)` stays put, which is what makes a wheel
    /// zoom feel pointed at the tile rather than at the window.
    pub fn zoomed_at(&self, x: f32, y: f32, factor: f32, ceiling: f32) -> Self {
        let scale = (self.scale * factor).clamp(Self::MIN_SCALE, ceiling);
        let (base_x, base_y) = self.unproject(x, y);
        Self {
            scale,
            origin_x: base_x - x / scale,
            origin_y: base_y - y / scale,
        }
    }

    /// The scale at which `rect` exactly fits the viewport.
    pub fn fit_scale(rect: Rect, area: Size<Pixels>) -> f32 {
        let width = area.width.as_f32().max(1.0);
        let height = area.height.as_f32().max(1.0);
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return Self::MIN_SCALE;
        }
        (width / rect.w).min(height / rect.h)
    }

    /// The region of the base layout the viewport currently shows.
    pub fn visible_base(&self, area: Size<Pixels>) -> Rect {
        let width = area.width.as_f32().max(1.0);
        let height = area.height.as_f32().max(1.0);
        Rect::new(
            self.origin_x,
            self.origin_y,
            width / self.scale,
            height / self.scale,
        )
    }

    /// Keep the viewport inside the layout: no empty margins, ever.
    pub fn clamped(self, area: Size<Pixels>) -> Self {
        let width = area.width.as_f32();
        let height = area.height.as_f32();
        let max_x = (width - width / self.scale).max(0.0);
        let max_y = (height - height / self.scale).max(0.0);
        Self {
            scale: self.scale,
            origin_x: self.origin_x.clamp(0.0, max_x),
            origin_y: self.origin_y.clamp(0.0, max_y),
        }
    }
}

/// A layout transition: the region the user is looking at, and where that same
/// region lands in the layout being switched to.
///
/// Every descending or ascending move is one region of the tree changing
/// address. Rather than moving a camera, each tile is drawn from where that
/// region *was* to where it *is*, so the tiles inside the directory that is
/// being entered grow into place and the ones being left slide out. That is
/// what makes "zoom in and descend" read as one motion: the newly visible
/// level is always larger than it was a moment ago, never smaller.
#[derive(Clone, Copy, Debug)]
pub struct LayoutTransition {
    /// The region in the old frame, in viewport coordinates.
    src: Rect,
    /// The same region in the new frame, in viewport coordinates.
    dst: Rect,
    started: Instant,
    duration: Duration,
}

impl LayoutTransition {
    fn new(src: Rect, dst: Rect) -> Self {
        // Longer pulls take a little longer, but never enough to feel slow.
        let ratio = if dst.w > 1.0 { src.w / dst.w } else { 1.0 };
        let magnitude = ratio.abs().max(1.0).log2().clamp(0.0, 4.0);
        Self {
            src,
            dst,
            started: Instant::now(),
            duration: Duration::from_millis(130 + (magnitude * 45.0) as u64),
        }
    }

    /// Where a rectangle in the new layout was, before the switch.
    fn origin_of(&self, rect: Rect) -> Rect {
        let scale = if self.dst.w > 0.0 {
            self.src.w / self.dst.w
        } else {
            1.0
        };
        Rect::new(
            (rect.x - self.dst.x).mul_add(scale, self.src.x),
            (rect.y - self.dst.y).mul_add(scale, self.src.y),
            rect.w * scale,
            rect.h * scale,
        )
    }

    /// The rectangle to draw at this instant, and whether the transition is
    /// still running.
    fn sample(&self, rect: Rect) -> (Rect, bool) {
        let elapsed =
            self.started.elapsed().as_secs_f32() / self.duration.as_secs_f32();
        if elapsed >= 1.0 {
            return (rect, false);
        }
        let eased = 1.0 - (1.0 - elapsed).powi(3);
        let from = self.origin_of(rect);
        let lerp = |a: f32, b: f32| (b - a).mul_add(eased, a);
        (
            Rect::new(
                lerp(from.x, rect.x),
                lerp(from.y, rect.y),
                lerp(from.w, rect.w),
                lerp(from.h, rect.h),
            ),
            true,
        )
    }
}

/// Layout for one (crumbs, area, options) combination.
struct LayoutCache {
    key: LayoutKey,
    tiles: Vec<Tile>,
}

#[derive(Clone, PartialEq)]
struct LayoutKey {
    crumbs: Vec<usize>,
    width: f32,
    height: f32,
    options: LayoutOptions,
}

/// Result of the removal run, summarised for the final screen.
#[derive(Clone, Debug, Default)]
pub struct RunSummary {
    pub total: usize,
    pub removed: u64,
    pub bytes: u64,
    pub failed: usize,
}

/// Everything the app knows and everything it can do.
pub struct Disktree {
    /// The directory the tree was scanned from.
    pub root_path: PathBuf,
    pub home: Option<PathBuf>,
    pub options: ScanOptions,
    pub tree: Option<Rc<Node>>,
    pub scan: Option<ScanHandle>,
    pub scan_epoch: u64,
    pub progress: ScanSnapshot,
    pub scan_error: Option<String>,

    pub screen: Screen,
    /// Path from the scanned root to the directory currently drawn.
    pub crumbs: Vec<usize>,
    pub selected: Option<Vec<usize>>,
    pub hovered: Option<Vec<usize>>,
    /// The pointer moved more recently than the keyboard navigated. Then the
    /// tile under the pointer is what Space, X and Enter act on; after an
    /// arrow or Tab it is the keyboard selection again.
    pub pointer_active: bool,
    pub view: View,
    /// The in-flight layout transition, if a level was just entered or left.
    pub transition: Option<LayoutTransition>,
    pub layout_options: LayoutOptions,
    cache: Option<LayoutCache>,

    /// Mouse position in treemap-local pixels, for hit-testing and the tooltip.
    pub pointer: Option<Point<Pixels>>,
    /// The treemap area in window space, recorded while painting.
    pub treemap_origin: Rc<Cell<Point<Pixels>>>,
    pub treemap_size: Rc<Cell<Size<Pixels>>>,

    pub marks: Marks,
    pub removal_mode: RemovalMode,
    pub trash_backend: TrashBackend,
    /// The permanent-deletion alert dialog is open. Trash needs no dialog: it
    /// is reversible, so it commits directly.
    pub confirm_open: bool,
    /// Focus owner for the alert dialog while it is open.
    pub confirm_focus: FocusHandle,
    /// Focus to move on the next occasion a window is in hand. Key handling
    /// has no window, and opening or closing the dialog must move focus.
    pub focus_request: Option<FocusTarget>,
    /// The window's `rem` in pixels, read each frame. The mosaic is laid out
    /// in pixels, so its header band and label thresholds are scaled by this
    /// to follow interface zoom like the rest of the interface.
    pub rem: f32,
    pub run: Option<RemovalHandle>,
    pub run_epoch: u64,
    pub run_summary: RunSummary,
    pub run_log: Vec<(PathBuf, Result<(), String>)>,
    pub space: Option<SpaceInfo>,
    /// Free space before the removal, for the honest "what did it actually
    /// free" number rather than the sum of what was marked.
    pub space_baseline: Option<SpaceInfo>,
    pub notice: Option<(String, Status)>,

    pub find: String,
    pub find_open: bool,
    pub show_help: bool,
    pub show_selection: bool,
    pub focus: FocusHandle,
}

impl Disktree {
    pub fn new(
        root_path: PathBuf,
        options: ScanOptions,
        depth: u32,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let space = space_info(&root_path).ok();
        let trash_backend = detect_trash_backend();
        let mut tree = Self {
            root_path,
            home,
            options,
            tree: None,
            scan: None,
            scan_epoch: 0,
            progress: ScanSnapshot::default(),
            scan_error: None,
            screen: Screen::Explore,
            crumbs: Vec::new(),
            selected: None,
            hovered: None,
            pointer_active: false,
            view: View::default(),
            transition: None,
            layout_options: LayoutOptions {
                max_depth: depth.clamp(1, 6),
                ..LayoutOptions::default()
            },
            cache: None,
            pointer: None,
            treemap_origin: Rc::new(Cell::new(Point::new(px(0.), px(0.)))),
            treemap_size: Rc::new(Cell::new(size(px(0.), px(0.)))),
            marks: Marks::default(),
            // Reversible by default whenever this machine has a trash: the
            // permanent path stays one choice away, behind a dialog.
            removal_mode: if trash_backend.is_available() {
                RemovalMode::Trash
            } else {
                RemovalMode::Permanent
            },
            trash_backend,
            confirm_open: false,
            confirm_focus: cx.focus_handle(),
            focus_request: None,
            rem: crate::ui::BASE_REM,
            run: None,
            run_epoch: 0,
            run_summary: RunSummary::default(),
            run_log: Vec::new(),
            space,
            space_baseline: None,
            notice: None,
            find: String::new(),
            find_open: false,
            show_help: false,
            show_selection: true,
            focus: cx.focus_handle(),
        };
        tree.start_scan(cx);
        Self::start_space_ticker(cx);
        tree
    }

    /// Build a view over a tree that is already known.
    ///
    /// Mirrors [`Self::new`] without starting a walk, so a test can hand the
    /// screens a tree it already has instead of waiting on a real one. A
    /// feature that opens an archived scan would use this too.
    #[cfg(test)]
    pub fn with_tree(
        root_path: PathBuf,
        tree: Node,
        options: ScanOptions,
        depth: u32,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let mut app = Self::new(root_path, options, depth, cx);
        app.marks.refresh(&app.root_path, &tree, app.options.metric);
        app.tree = Some(Rc::new(tree));
        app.cache = None;
        app.select_largest(cx);
        app
    }

    /// Select the largest entry of the current root, so the selection line, the
    /// tooltip and the mark key all have something to act on from the first
    /// frame. The largest entry is also the answer to "what is eating my disk"
    /// most of the time.
    fn select_largest(&mut self, cx: &mut Context<'_, Self>) {
        if self.selected.is_some() {
            return;
        }
        if let Some(index) = self.current().and_then(Node::largest_child) {
            self.selected = Some(vec![index]);
            cx.notify();
        }
    }

    // ── scanning ────────────────────────────────────────────────────────

    /// Start a fresh scan, abandoning any walk still in progress.
    pub fn start_scan(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(scan) = &self.scan {
            scan.cancel();
        }
        self.scan_epoch += 1;
        let epoch = self.scan_epoch;
        self.progress = ScanSnapshot::default();
        self.scan_error = None;
        self.tree = None;
        self.crumbs.clear();
        self.selected = None;
        self.hovered = None;
        self.view = View::default();
        self.cache = None;
        self.scan = Some(ScanHandle::spawn(
            self.root_path.clone(),
            self.options.clone(),
        ));
        Self::poll_scan(epoch, cx);
        cx.notify();
    }

    fn poll_scan(epoch: u64, cx: &Context<'_, Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(110))
                    .await;
                let keep_going = this
                    .update(cx, |this, cx| this.poll_scan_once(epoch, cx))
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    /// Drain the scan channel once. Returns whether the poller should keep
    /// ticking. Driven by the poll task above, and by the tests, which cannot
    /// wait on the test clock while a real worker thread walks a directory.
    pub(crate) fn poll_scan_once(
        &mut self,
        epoch: u64,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if epoch != self.scan_epoch {
            return false;
        }
        let Some(scan) = &self.scan else {
            return false;
        };
        self.progress = scan.progress.snapshot();
        let Some(outcome) = scan.poll() else {
            cx.notify();
            return true;
        };
        match outcome {
            Ok(node) => {
                let metric = self.options.metric;
                self.marks.refresh(&self.root_path, &node, metric);
                self.tree = Some(Rc::new(node));
                self.cache = None;
                self.keep_selection_valid();
                self.select_largest(cx);
            }
            Err(error) => self.scan_error = Some(error.to_string()),
        }
        self.scan = None;
        self.progress.finished = true;
        cx.notify();
        false
    }

    fn keep_selection_valid(&mut self) {
        let Some(tree) = &self.tree else {
            return;
        };
        if tree.resolve(&self.crumbs).is_none() {
            self.crumbs.clear();
        }
        if let Some(selected) = self.selected.clone()
            && tree.resolve(&selected).is_none()
        {
            self.selected = None;
        }
    }

    /// Space changes under us all the time; poll it so the meter is live
    /// without repainting when nothing moved.
    fn start_space_ticker(cx: &Context<'_, Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                let interval = this
                    .update(cx, |this, _| {
                        if this.screen == Screen::Running {
                            Duration::from_millis(150)
                        } else {
                            Duration::from_millis(1200)
                        }
                    })
                    .unwrap_or(Duration::from_millis(1200));
                cx.background_executor().timer(interval).await;
                let ok = this
                    .update(cx, |this, cx| {
                        let path = this.root_path.clone();
                        let fresh = space_info(&path).ok();
                        if fresh != this.space {
                            this.space = fresh;
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !ok {
                    break;
                }
            }
        })
        .detach();
    }

    // ── navigation ──────────────────────────────────────────────────────

    pub fn tree(&self) -> Option<&Node> {
        self.tree.as_deref()
    }

    /// The node the treemap is currently rooted at.
    pub fn current(&self) -> Option<&Node> {
        self.tree
            .as_ref()
            .and_then(|tree| tree.resolve(&self.crumbs))
    }

    pub fn node_at(&self, crumbs: &[usize]) -> Option<&Node> {
        self.tree.as_ref().and_then(|tree| tree.resolve(crumbs))
    }

    pub fn path_at(&self, crumbs: &[usize]) -> Option<PathBuf> {
        let tree = self.tree.as_ref()?;
        Some(path_of(&self.root_path, tree, crumbs))
    }

    /// Path of the directory currently drawn.
    pub fn current_path(&self) -> PathBuf {
        self.path_at(&self.crumbs)
            .unwrap_or_else(|| self.root_path.clone())
    }

    /// Breadcrumb labels from the scanned root to the current directory.
    pub fn breadcrumbs(&self) -> Vec<(String, Vec<usize>)> {
        let mut trail = vec![(
            display_path(&self.root_path, self.home.as_deref()),
            Vec::new(),
        )];
        let Some(tree) = &self.tree else {
            return trail;
        };
        let mut crumbs: Vec<usize> = Vec::new();
        for &index in &self.crumbs {
            let Some(node) =
                tree.resolve(&crumbs).and_then(|node| node.child(index))
            else {
                break;
            };
            crumbs.push(index);
            trail.push((node.name.to_string(), crumbs.clone()));
        }
        trail
    }

    /// Descend into the selected tile, or into the largest child of the
    /// current root when nothing is selected.
    pub fn descend(&mut self, cx: &mut Context<'_, Self>) {
        let target = match self.selected.clone() {
            // Enter what is selected, however deep: a directory opens itself,
            // a file opens the directory holding it.
            Some(selected) if selected.len() > self.crumbs.len() => {
                if self.node_at(&selected).is_some_and(Node::is_dir) {
                    selected
                } else {
                    selected[..selected.len() - 1].to_vec()
                }
            }
            // Nothing below the root is selected: the largest entry.
            _ => match self.current().and_then(Node::largest_child) {
                Some(index) => {
                    let mut crumbs = self.crumbs.clone();
                    crumbs.push(index);
                    crumbs
                }
                None => return,
            },
        };
        let from = self.tile_body(&target).map(|rect| self.view.project(rect));
        self.enter(target, from, cx);
    }

    /// Go inside one child of the current root, so that it fills the viewport.
    ///
    /// `from` is where the child was on screen, when the caller knows it.
    /// Make `target` — any directory below the current root — the root.
    ///
    /// `from` is where that directory's contents were on screen, so the
    /// transition grows them from exactly there into the full viewport.
    fn enter(
        &mut self,
        target: Vec<usize>,
        from: Option<Rect>,
        cx: &mut Context<'_, Self>,
    ) {
        if target.len() <= self.crumbs.len()
            || !target.starts_with(&self.crumbs)
        {
            return;
        }
        let Some(node) = self.node_at(&target) else {
            return;
        };
        if !node.is_dir() || node.children.is_empty() {
            return;
        }
        self.selected = Some(target.clone());
        self.crumbs = target;
        self.forget_hover();
        self.cache = None;
        let area = self.treemap_size.get();
        let src = from.unwrap_or_else(|| self.view.visible_base(area));
        self.view = View::IDENTITY;
        let dst = Rect::new(
            0.0,
            0.0,
            area.width.as_f32().max(1.0),
            area.height.as_f32().max(1.0),
        );
        self.transition = Some(LayoutTransition::new(src, dst));
        cx.notify();
    }

    /// Where a drawn directory's contents sit: its tile below the name band.
    /// This, not the whole tile, is the region its children occupy, so it is
    /// what a transition into or out of it has to map.
    pub fn tile_body(&mut self, crumbs: &[usize]) -> Option<Rect> {
        let tile =
            self.layout()?.iter().find(|tile| tile.crumbs() == crumbs)?;
        Some(match tile.header {
            Some(header) => Rect::new(
                tile.rect.x,
                header.bottom(),
                tile.rect.w,
                tile.rect.bottom() - header.bottom(),
            ),
            None => tile.rect,
        })
    }

    /// The deepest drawn directory under a viewport point that has contents
    /// to show: what zooming at that point is zooming into.
    fn zoom_target(&mut self, x: f32, y: f32) -> Option<Vec<usize>> {
        let hovered = self.tile_at(x, y)?;
        let root = self.crumbs.len();
        (root + 1..=hovered.len())
            .rev()
            .map(|length| hovered[..length].to_vec())
            .find(|crumbs| {
                self.node_at(crumbs).is_some_and(|node| {
                    node.is_dir() && !node.children.is_empty()
                }) && self.tile_body(crumbs).is_some()
            })
    }

    /// Ascend to the parent directory, keeping the directory we came from in
    /// view so the motion reads as zooming out.
    pub fn ascend(&mut self, cx: &mut Context<'_, Self>) {
        let Some(parent_crumbs) = self.parent_crumbs() else {
            return;
        };
        // The region we are looking at now, and where it sits in the layout
        // we are going back to.
        let area = self.treemap_size.get();
        let child_crumbs = self.crumbs.clone();
        let src = self.view.visible_base(area);
        self.crumbs.clone_from(&parent_crumbs);
        self.forget_hover();
        self.cache = None;
        self.selected = Some(parent_crumbs);
        self.view = View::IDENTITY;
        self.transition = self
            .tile_body(&child_crumbs)
            .filter(|rect| rect.w > 1.0 && rect.h > 1.0)
            .map(|dst| LayoutTransition::new(src, dst));
        cx.notify();
    }

    /// The crumbs of the parent of the current root, if any.
    pub fn parent_crumbs(&self) -> Option<Vec<usize>> {
        if self.crumbs.is_empty() {
            None
        } else {
            Some(self.crumbs[..self.crumbs.len() - 1].to_vec())
        }
    }

    /// Jump straight to a crumb from the breadcrumb bar.
    ///
    /// A jump can skip several levels, so there is no single region to move:
    /// it lands immediately, the way selecting a folder does.
    pub fn go_to(&mut self, crumbs: Vec<usize>, cx: &mut Context<'_, Self>) {
        self.crumbs.clone_from(&crumbs);
        self.selected = Some(crumbs);
        self.forget_hover();
        self.view = View::IDENTITY;
        self.transition = None;
        self.cache = None;
        cx.notify();
    }

    /// Select the tile at `crumbs` without changing the root.
    pub fn select(
        &mut self,
        crumbs: Option<Vec<usize>>,
        cx: &mut Context<'_, Self>,
    ) {
        self.selected = crumbs;
        cx.notify();
    }

    /// Move the selection geometrically, falling back to the parent at an edge.
    /// The tile a key acts on: under the pointer if the pointer moved last,
    /// otherwise the keyboard selection.
    pub fn action_target(&self) -> Option<Vec<usize>> {
        if self.pointer_active {
            self.hovered.clone().or_else(|| self.selected.clone())
        } else {
            self.selected.clone()
        }
    }

    /// Make the pointed-at tile the selection before a key acts, so marking,
    /// opening and arrow movement all start from what the user is looking at.
    fn adopt_pointer_target(&mut self) {
        if self.pointer_active
            && let Some(hovered) = self.hovered.clone()
        {
            self.selected = Some(hovered);
        }
    }

    /// After the layout changes under a still pointer, its hover is stale
    /// until the pointer moves again.
    fn forget_hover(&mut self) {
        self.hovered = None;
        self.pointer_active = false;
    }

    pub fn move_selection(
        &mut self,
        direction: Direction,
        cx: &mut Context<'_, Self>,
    ) {
        self.pointer_active = false;
        let area = self.treemap_size.get();
        let _ = area;
        let Some(tiles) = self.layout().map(<[Tile]>::to_vec) else {
            return;
        };
        let Some(current) = self.selected.clone() else {
            if let Some(first) = tiles.first() {
                let crumbs = first.crumbs().to_vec();
                self.select(Some(crumbs), cx);
            }
            return;
        };
        let Some(from) = tiles
            .iter()
            .find(|tile| tile.crumbs() == current.as_slice())
            .map(|tile| self.view.project(tile.rect))
        else {
            return;
        };

        // Same-depth neighbours only: stepping into a child by arrow key would
        // make the depth of the selection impossible to predict.
        let depth = current.len();
        let mut best: Option<(f32, Vec<usize>)> = None;
        for tile in &tiles {
            let crumbs = tile.crumbs();
            if crumbs.len() != depth || crumbs == current.as_slice() {
                continue;
            }
            let rect = self.view.project(tile.rect);
            let Some(gap) = direction.gap(&from, &rect) else {
                continue;
            };
            let offset = direction.offset(&from, &rect);
            let score = offset.mul_add(2.5, gap);
            if best.as_ref().is_none_or(|(existing, _)| score < *existing) {
                best = Some((score, crumbs.to_vec()));
            }
        }

        if let Some((_, crumbs)) = best {
            self.select(Some(crumbs), cx);
        } else if direction.is_backwards()
            && let Some(parent) = self.parent_crumbs()
        {
            self.select(Some(parent), cx);
        }
    }

    /// Select the next sibling by rank, which is next-largest by the active
    /// metric. Scanning a directory for space is exactly this walk.
    pub fn cycle_sibling(&mut self, step: isize, cx: &mut Context<'_, Self>) {
        self.pointer_active = false;
        let siblings = self.ranked_siblings();
        if siblings.is_empty() {
            return;
        }
        let current = self.selected.as_ref().and_then(|selected| {
            siblings
                .iter()
                .position(|crumbs| crumbs.as_slice() == selected.as_slice())
        });
        let next = match current {
            Some(index) => {
                let len = siblings.len().cast_signed();
                (((index.cast_signed() + step) % len + len) % len) as usize
            }
            None if step >= 0 => 0,
            None => siblings.len() - 1,
        };
        self.select(Some(siblings[next].clone()), cx);
    }

    fn ranked_siblings(&self) -> Vec<Vec<usize>> {
        let parent = self.selected.as_ref().map_or_else(
            || self.crumbs.clone(),
            |selected| {
                if selected.len() <= self.crumbs.len() {
                    self.crumbs.clone()
                } else {
                    selected[..selected.len() - 1].to_vec()
                }
            },
        );
        let Some(node) = self.node_at(&parent) else {
            return Vec::new();
        };
        (0..node.children.len())
            .map(|index| {
                let mut crumbs = parent.clone();
                crumbs.push(index);
                crumbs
            })
            .collect()
    }

    /// Mark or unmark the current selection.
    pub fn toggle_mark_selected(&mut self, cx: &mut Context<'_, Self>) {
        let Some(crumbs) = self.selected.clone() else {
            return;
        };
        if crumbs.is_empty() {
            self.notice = Some((
                "the scanned root cannot be removed; open a directory first"
                    .into(),
                Status::Warning,
            ));
            cx.notify();
            return;
        }
        self.toggle_mark(&crumbs, cx);
    }

    pub fn toggle_mark(
        &mut self,
        crumbs: &[usize],
        cx: &mut Context<'_, Self>,
    ) {
        let Some(target) = self.target_at(crumbs) else {
            return;
        };
        let marked_now = self.marks.toggle(target);
        self.notice = None;
        if !marked_now {
            cx.notify();
            return;
        }
        // A mark inside something already marked is redundant, and saying so
        // now is friendlier than showing it as redundant on the review screen.
        let marked = self.marks.items().last().cloned();
        if let Some(marked) = marked
            && let Some(parent) = self.marks.items().iter().find(|item| {
                item.path != marked.path && marked.path.starts_with(&item.path)
            })
        {
            let parent = parent.path.clone();
            self.space_baseline = self.space_baseline.or(self.space);
            self.notice = Some((
                format!(
                    "{} is already inside the marked {}",
                    display_path(&marked.path, self.home.as_deref()),
                    display_path(&parent, self.home.as_deref())
                ),
                Status::Warning,
            ));
        }
        cx.notify();
    }

    pub fn target_at(&self, crumbs: &[usize]) -> Option<Target> {
        let node = self.node_at(crumbs)?;
        let path = self.path_at(crumbs)?;
        Some(Target {
            hidden: is_hidden(&path),
            path,
            bytes: node.value(self.options.metric),
            is_dir: node.is_dir(),
        })
    }

    pub fn unmark(&mut self, path: &Path, cx: &mut Context<'_, Self>) {
        self.marks.remove(path);
        cx.notify();
    }

    pub fn clear_marks(&mut self, cx: &mut Context<'_, Self>) {
        self.marks.clear();
        self.notice = None;
        cx.notify();
    }

    /// The plan the review screen shows and the removal runs.
    pub fn plan(&self) -> Plan {
        plan(self.marks.items(), &self.root_path)
    }

    // ── layout and hit-testing ──────────────────────────────────────────

    /// Resolve everything the mosaic needs for this frame.
    ///
    /// Runs once per render, before the paint callbacks: painting then only has
    /// to draw, and the marked/hidden/selected state of every tile is decided
    /// here, where the tree and the marks are both at hand.
    pub fn prepare(&mut self) -> Mosaic {
        let metric = self.options.metric;
        let view = self.view;
        let hovered = self.hovered.clone();
        let selected = self.selected.clone();

        // Marks are paths; the mosaic thinks in crumbs. Resolve once per frame
        // rather than building a path for every tile.
        let mut marked: FxHashSet<Vec<usize>> = FxHashSet::default();
        for item in self.marks.items() {
            if let Some(crumbs) = self.crumbs_for_path(&item.path) {
                marked.insert(crumbs);
            }
        }
        let mut covered: FxHashSet<Vec<usize>> = FxHashSet::default();
        for crumbs in &marked {
            if (1..crumbs.len())
                .any(|length| marked.contains(&crumbs[..length]))
            {
                covered.insert(crumbs.clone());
            }
        }

        let Some(tiles) = self.layout().map(<[Tile]>::to_vec) else {
            return Mosaic {
                view,
                ..Mosaic::default()
            };
        };

        // Colours are keyed to depth-from-the-scanned-root, not depth-from-this
        // view: the layout root is re-based every time the view descends, so a
        // view-relative key would recolour every tile as you walk in. The view
        // root's own absolute depth is the breadcrumb count.
        let base_depth = u32::try_from(self.crumbs.len()).unwrap_or(u32::MAX);
        let mut decorations = Vec::with_capacity(tiles.len());
        let mut labels = Vec::new();
        for tile in &tiles {
            let crumbs = tile.crumbs();
            let hidden = self.crumbs_are_hidden(crumbs);
            let is_marked = marked.contains(crumbs);
            let is_covered = covered.contains(crumbs);
            decorations.push(TileDeco {
                rect: self.animated_rect(tile.rect),
                header: tile.header.map(|header| self.animated_rect(header)),
                depth: tile.depth,
                color_depth: base_depth.saturating_add(tile.depth),
                hidden,
                marked: is_marked,
                covered: is_covered,
                hovered: hovered.as_deref() == Some(crumbs),
                selected: selected.as_deref() == Some(crumbs),
            });

            // Labels are chosen in screen space: zooming in makes room for more
            // of them, which is the point of zooming in.
            let drawn = self.animated_rect(tile.rect);
            let screen = view.project(
                tile.header
                    .map_or(drawn, |header| self.animated_rect(header)),
            );
            if screen.w < LABEL_MIN_W_REMS * self.rem
                || screen.h < LABEL_MIN_H_REMS * self.rem
            {
                continue;
            }
            match &tile.kind {
                TileKind::Node { crumbs } => {
                    let Some(node) = self.node_at(crumbs) else {
                        continue;
                    };
                    labels.push(Label {
                        text: node.name.to_string(),
                        rect: self.animated_rect(tile.rect),
                        header: tile
                            .header
                            .map(|header| self.animated_rect(header)),
                        color_depth: base_depth.saturating_add(tile.depth),
                        marked: is_marked || is_covered,
                        size_text: crate::widgets::short_value(node, metric),
                    });
                }
                TileKind::Others { count, .. } => labels.push(Label {
                    text: format!("+{count} more"),
                    rect: self.animated_rect(tile.rect),
                    header: None,
                    color_depth: base_depth.saturating_add(tile.depth),
                    marked: false,
                    size_text: String::new(),
                }),
            }
        }

        labels.sort_by(|left, right| {
            let left_area = left.rect.w * left.rect.h;
            let right_area = right.rect.w * right.rect.h;
            right_area.total_cmp(&left_area)
        });
        labels.truncate(MAX_LABELS);

        Mosaic {
            tiles: decorations,
            labels,
            view,
        }
    }

    /// Crumbs for an absolute path, if the tree still contains it.
    #[allow(
        clippy::unnecessary_wraps,
        reason = "a marked path may have been removed from the tree already"
    )]
    pub fn crumbs_for_path(&self, path: &Path) -> Option<Vec<usize>> {
        // The path is relative to the scanned root, so the walk starts there,
        // not at the directory currently drawn.
        let relative = path.strip_prefix(&self.root_path).ok()?;
        let tree = self.tree.clone()?;
        let mut node: &Node = &tree;
        let mut crumbs = Vec::new();
        for component in relative.components() {
            let name = component.as_os_str().to_string_lossy();
            let index = node
                .children
                .iter()
                .position(|child| child.name.as_ref() == name)?;
            crumbs.push(index);
            node = node.child(index)?;
        }
        Some(crumbs)
    }

    /// Whether any component of the path to `crumbs` is a dotfile.
    fn crumbs_are_hidden(&self, crumbs: &[usize]) -> bool {
        let Some(tree) = &self.tree else {
            return false;
        };
        tree.resolve_chain(crumbs)
            .iter()
            .any(|node| node.name.starts_with('.'))
    }

    /// Tiles for the current root and viewport size, computed once per change.
    pub fn layout(&mut self) -> Option<&[Tile]> {
        // A 17 px band at the default rem, scaled so zoom keeps the band's
        // relationship to the label inside it.
        self.layout_options.header = HEADER_REMS * self.rem;
        let area = self.treemap_size.get();
        let key = LayoutKey {
            crumbs: self.crumbs.clone(),
            width: area.width.as_f32().round(),
            height: area.height.as_f32().round(),
            options: self.layout_options.clone(),
        };
        if key.width < 1.0 || key.height < 1.0 {
            return None;
        }
        let stale = self.cache.as_ref().is_none_or(|cache| cache.key != key);
        if stale {
            let tree = self.tree.clone()?;
            let node = tree.resolve(&self.crumbs)?;
            let rect = Rect::new(0.0, 0.0, key.width, key.height);
            let tiles = layout(
                node,
                &key.crumbs,
                rect,
                self.options.metric,
                &key.options,
            );
            self.cache = Some(LayoutCache { key, tiles });
        }
        self.cache.as_ref().map(|cache| cache.tiles.as_slice())
    }

    /// Base-space rect of the tile at `crumbs`, if it is currently drawn.
    #[cfg(test)]
    pub fn tile_rect(&mut self, crumbs: &[usize]) -> Option<Rect> {
        self.layout()?
            .iter()
            .find(|tile| tile.crumbs() == crumbs)
            .map(|tile| tile.rect)
    }

    /// The deepest tile under a viewport point.
    pub fn tile_at(&mut self, x: f32, y: f32) -> Option<Vec<usize>> {
        let (base_x, base_y) = self.view.unproject(x, y);
        let tiles = self.layout()?;
        hit(tiles, base_x, base_y).map(|tile| tile.crumbs().to_vec())
    }

    // ── view ────────────────────────────────────────────────────────────

    /// Zoom toward a point.
    ///
    /// When `descend` is set the wheel stops magnifying at the point where the
    /// directory under the pointer exactly fits the viewport, and the next
    /// notch goes inside it. That ceiling is what keeps the two gestures
    /// continuous: at the moment the level changes, the tiles inside the
    /// directory are already as large as they will be, so entering makes them
    /// grow rather than shrink.
    pub fn zoom_at(
        &mut self,
        x: f32,
        y: f32,
        factor: f32,
        descend: bool,
        cx: &mut Context<'_, Self>,
    ) {
        let area = self.treemap_size.get();

        // At the bottom of the zoom, zooming out goes up a level.
        if factor < 1.0 && self.view.scale <= View::MIN_SCALE + f32::EPSILON {
            if descend {
                self.ascend(cx);
            }
            return;
        }

        // One directory decides both how far the wheel magnifies and where it
        // then goes: the deepest one under the pointer. At the ceiling its
        // contents fill the view, so going inside continues the same motion.
        let target = if descend {
            self.zoom_target(x, y)
        } else {
            None
        };
        let body = target.as_ref().and_then(|crumbs| self.tile_body(crumbs));
        let ceiling = body
            .map_or(View::MAX_SCALE, |rect| View::fit_scale(rect, area))
            .clamp(View::MIN_SCALE, View::MAX_SCALE);

        if factor > 1.0
            && self.view.scale >= ceiling - f32::EPSILON
            && let Some(target) = target
        {
            let from = body.map(|rect| self.view.project(rect));
            self.enter(target, from, cx);
            return;
        }

        self.view = self.view.zoomed_at(x, y, factor, ceiling).clamped(area);
        cx.notify();
    }

    pub fn reset_view(&mut self, cx: &mut Context<'_, Self>) {
        self.view = View::default();
        cx.notify();
    }

    /// Change how many levels are drawn, which is the other meaning of zoom in
    /// a treemap: seeing further in without changing what is on screen.
    pub fn adjust_depth(&mut self, step: i32, cx: &mut Context<'_, Self>) {
        let depth = (self.layout_options.max_depth.cast_signed() + step)
            .clamp(1, 6) as u32;
        self.layout_options.max_depth = depth;
        self.cache = None;
        cx.notify();
    }

    pub fn toggle_metric(&mut self, cx: &mut Context<'_, Self>) {
        self.options.metric = self.options.metric.toggled();
        if let Some(tree) = &self.tree {
            let mut tree = (**tree).clone();
            disktree_core::tree::aggregate(&mut tree, self.options.metric);
            self.tree = Some(Rc::new(tree));
            let metric = self.options.metric;
            self.marks.refresh(
                &self.root_path,
                self.tree.as_ref().unwrap(),
                metric,
            );
        }
        self.cache = None;
        cx.notify();
    }

    // ── removal ─────────────────────────────────────────────────────────

    /// The review screen's commit: move to the trash at once, or ask first for
    /// a permanent deletion, which cannot be undone.
    pub fn commit(&mut self, cx: &mut Context<'_, Self>) {
        if self.plan().is_empty() {
            return;
        }
        match self.removal_mode {
            RemovalMode::Trash => self.begin_removal(cx),
            RemovalMode::Permanent => {
                self.confirm_open = true;
                self.focus_request = Some(FocusTarget::Dialog);
                cx.notify();
            }
        }
    }

    /// The alert dialog's `Delete`.
    pub fn confirm_delete(&mut self, cx: &mut Context<'_, Self>) {
        self.confirm_open = false;
        self.focus_request = Some(FocusTarget::Root);
        self.begin_removal(cx);
    }

    /// The alert dialog's `Cancel`, or Escape.
    pub fn cancel_delete(&mut self, cx: &mut Context<'_, Self>) {
        self.confirm_open = false;
        self.focus_request = Some(FocusTarget::Root);
        cx.notify();
    }

    /// Move focus if something asked for it. Called wherever a window is in
    /// hand: the key listener and the click listeners.
    pub fn apply_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match self.focus_request.take() {
            Some(FocusTarget::Root) => window.focus(&self.focus, cx),
            Some(FocusTarget::Dialog) => window.focus(&self.confirm_focus, cx),
            None => {}
        }
    }

    /// `ctrl =`, `ctrl -` and `ctrl 0`: interface zoom. Changes the window's
    /// `rem`, which every size in this app is expressed in, so hierarchy and
    /// spacing keep their proportions at every step.
    pub fn zoom_interface(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
    ) -> bool {
        let keystroke = &event.keystroke;
        if !(keystroke.modifiers.control || keystroke.modifiers.platform) {
            return false;
        }
        let steps = crate::ui::ZOOM_STEPS;
        let current = window.rem_size().as_f32() / crate::ui::BASE_REM;
        let index = steps
            .iter()
            .position(|step| (step - current).abs() < 0.01)
            .unwrap_or(2);
        let next = match keystroke.key.as_str() {
            "=" | "+" => (index + 1).min(steps.len() - 1),
            "-" => index.saturating_sub(1),
            "0" => 2,
            _ => return false,
        };
        window.set_rem_size(px(crate::ui::BASE_REM * steps[next]));
        self.cache = None;
        true
    }

    pub fn begin_removal(&mut self, cx: &mut Context<'_, Self>) {
        let plan = self.plan();
        if plan.is_empty() {
            self.notice = Some(("nothing is marked".into(), Status::Warning));
            cx.notify();
            return;
        }
        self.space_baseline = self.space.or(self.space_baseline);
        self.run_epoch += 1;
        let epoch = self.run_epoch;
        self.run_summary = RunSummary {
            total: plan.targets.len(),
            ..RunSummary::default()
        };
        self.run_log.clear();
        self.screen = Screen::Running;
        self.run = Some(disktree_core::removal::spawn(plan, self.removal_mode));
        Self::poll_removal(epoch, cx);
        cx.notify();
    }

    fn poll_removal(epoch: u64, cx: &Context<'_, Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(60))
                    .await;
                let keep_going = this
                    .update(cx, |this, cx| this.poll_removal_once(epoch, cx))
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    /// Drain the removal channel once. Returns whether the run should keep
    /// being polled. Driven by the poll task above, and by the tests, which
    /// cannot wait on the test clock while a real worker thread runs.
    pub(crate) fn poll_removal_once(
        &mut self,
        epoch: u64,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if epoch != self.run_epoch {
            return false;
        }
        let Some(run) = &self.run else {
            return false;
        };
        let mut finished = false;
        while let Some(event) = run.poll() {
            match event {
                RemovalEvent::Start { total } => self.run_summary.total = total,
                RemovalEvent::Item {
                    path,
                    bytes,
                    outcome,
                } => {
                    if outcome.is_ok() {
                        self.run_summary.removed += 1;
                        self.run_summary.bytes += bytes;
                    } else {
                        self.run_summary.failed += 1;
                    }
                    self.run_log.push((path, outcome));
                }
                RemovalEvent::Done {
                    removed,
                    bytes,
                    failed,
                } => {
                    self.run_summary.removed = removed;
                    self.run_summary.bytes = bytes;
                    self.run_summary.failed = failed;
                    finished = true;
                }
            }
        }
        if finished {
            self.run = None;
            self.marks.clear();
            self.screen = Screen::Done;
            // The tree on screen is now wrong; start over rather than leaving
            // numbers that include what was just removed.
            self.start_scan(cx);
        }
        cx.notify();
        !finished
    }

    /// Descend to a path found by the search field.
    pub fn jump_to_match(&mut self, cx: &mut Context<'_, Self>) {
        let needle = self.find.trim().to_lowercase();
        if needle.is_empty() {
            return;
        }
        let Some(tree) = self.tree.clone() else {
            return;
        };
        let Some((crumbs, _)) = tree.find(&needle) else {
            self.notice =
                Some((format!("no entry matches {needle}"), Status::Warning));
            cx.notify();
            return;
        };
        let parent = crumbs[..crumbs.len().saturating_sub(1)].to_vec();
        self.go_to(parent, cx);
        self.selected = Some(crumbs);
        self.notice = None;
        cx.notify();
    }

    // ── input ───────────────────────────────────────────────────────────

    /// Handle a key press. Returns whether the directory on screen changed,
    /// which is what keeps the window title honest.
    pub fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let before = self.crumbs.clone();
        self.dispatch_key(event, cx);
        self.crumbs != before
    }

    /// Every binding, in reading order of the hint bar, so the keys and the
    /// documented list cannot drift apart.
    fn dispatch_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<'_, Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let control = event.keystroke.modifiers.control;
        let shift = event.keystroke.modifiers.shift;

        // The alert dialog owns Enter and Escape while it is open; a key that
        // bubbles up to here must not also act on the screen behind it.
        if self.confirm_open {
            return;
        }

        if self.show_help {
            if matches!(key, "escape" | "?" | "/" | "q") {
                self.show_help = false;
                cx.notify();
            }
            return;
        }

        // The search field takes the keyboard while it is open. It is a field
        // of three keys on purpose: no editor state to keep in sync with the
        // tree, and Escape always means "give the keyboard back".
        if self.screen == Screen::Explore && self.find_open {
            match key {
                "escape" => {
                    self.find_open = false;
                    self.find.clear();
                }
                "enter" => {
                    self.find_open = false;
                    self.jump_to_match(cx);
                }
                "backspace" => {
                    self.find.pop();
                }
                _ => {
                    // Some platforms report only `key` for a character and
                    // leave `key_char` empty; a one-character key is a
                    // character either way.
                    let typed = event
                        .keystroke
                        .key_char
                        .as_deref()
                        .unwrap_or(event.keystroke.key.as_str());
                    if !control && typed.chars().count() == 1 {
                        self.find.push_str(typed);
                    }
                }
            }
            cx.notify();
            return;
        }

        match self.screen {
            Screen::Explore => self.on_explore_key(key, control, shift, cx),
            Screen::Review => self.on_review_key(key, cx),
            Screen::Running => {
                if key == "escape"
                    && let Some(run) = &self.run
                {
                    run.cancel();
                    self.notice = Some((
                        "stopping after the current item".into(),
                        Status::Warning,
                    ));
                    cx.notify();
                }
            }
            Screen::Done => {
                if matches!(key, "escape" | "enter") {
                    self.screen = Screen::Explore;
                    cx.notify();
                }
            }
        }
    }

    fn on_explore_key(
        &mut self,
        key: &str,
        control: bool,
        shift: bool,
        cx: &mut Context<'_, Self>,
    ) {
        // Keys that act on "the current tile" start from the pointer when it
        // moved last.
        if matches!(
            key,
            "space"
                | "x"
                | "enter"
                | "tab"
                | "left"
                | "right"
                | "up"
                | "down"
                | "h"
                | "j"
                | "k"
                | "l"
        ) {
            self.adopt_pointer_target();
        }
        match key {
            "/" | "s" if !control => {
                self.find_open = true;
                cx.notify();
            }
            "enter" => self.descend(cx),
            "right" | "l" if !control => {
                self.move_selection(Direction::Right, cx);
            }
            "left" | "h" if !control => {
                self.move_selection(Direction::Left, cx);
            }
            "up" | "k" if !control => self.move_selection(Direction::Up, cx),
            "down" | "j" if !control => {
                self.move_selection(Direction::Down, cx);
            }
            "backspace" | "u" if !control => self.ascend(cx),
            "escape" => {
                if self.selected.is_some() {
                    self.selected = None;
                } else {
                    self.ascend(cx);
                }
                cx.notify();
            }
            "space" => self.toggle_mark_selected(cx),
            "x" if !control => self.toggle_mark_selected(cx),
            "tab" => self.cycle_sibling(if shift { -1 } else { 1 }, cx),
            "c" if !control => {
                if self.marks.is_empty() {
                    self.notice = Some((
                        "mark something first: space marks the selected tile"
                            .into(),
                        Status::Warning,
                    ));
                } else {
                    self.screen = Screen::Review;
                }
                cx.notify();
            }
            "[" => self.adjust_depth(-1, cx),
            "]" => self.adjust_depth(1, cx),
            "-" => {
                self.zoom_at(
                    self.half_width(),
                    self.half_height(),
                    1.0 / 1.25,
                    false,
                    cx,
                );
            }
            "=" | "+" => {
                self.zoom_at(
                    self.half_width(),
                    self.half_height(),
                    1.25,
                    false,
                    cx,
                );
            }
            "0" => self.reset_view(cx),
            "t" if !control => self.toggle_metric(cx),
            "r" if !control => self.start_scan(cx),
            "i" if !control => {
                self.options.include_hidden = !self.options.include_hidden;
                self.start_scan(cx);
            }
            "d" if !control => {
                self.options.apparent_size = !self.options.apparent_size;
                self.start_scan(cx);
            }
            "p" if !control => {
                self.show_selection = !self.show_selection;
                cx.notify();
            }
            "?" => {
                self.show_help = true;
                cx.notify();
            }
            "q" if !control => cx.quit(),
            _ => {}
        }
    }

    fn on_review_key(&mut self, key: &str, cx: &mut Context<'_, Self>) {
        match key {
            "escape" => {
                self.screen = Screen::Explore;
                cx.notify();
            }
            "enter" => self.commit(cx),
            "!" => self.clear_marks(cx),
            "p" => {
                self.removal_mode = RemovalMode::Permanent;
                cx.notify();
            }
            "m" => {
                self.removal_mode = RemovalMode::Trash;
                cx.notify();
            }
            "?" => {
                self.show_help = true;
                cx.notify();
            }
            _ => {}
        }
    }

    fn half_width(&self) -> f32 {
        self.treemap_size.get().width.as_f32() / 2.0
    }

    fn half_height(&self) -> f32 {
        self.treemap_size.get().height.as_f32() / 2.0
    }

    /// Mouse moved over the treemap.
    pub fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<'_, Self>,
    ) {
        let origin = self.treemap_origin.get();
        let local = Point::new(
            event.position.x - origin.x,
            event.position.y - origin.y,
        );
        self.pointer = Some(local);
        self.pointer_active = true;
        let previous = self.hovered.clone();
        self.hovered = self.tile_at(local.x.as_f32(), local.y.as_f32());
        // The cursor tooltip is positioned from `pointer`, so a move *within*
        // one tile still has to repaint — otherwise the tooltip sticks where
        // the tile was first entered until the hover target changes.
        if self.hovered != previous || self.hovered.is_some() {
            cx.notify();
        }
    }

    pub fn on_mouse_leave(&mut self, cx: &mut Context<'_, Self>) {
        self.pointer = None;
        self.pointer_active = false;
        if self.hovered.take().is_some() {
            cx.notify();
        }
    }

    /// Click: select, then act on a repeat click, like a file manager.
    pub fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        cx: &mut Context<'_, Self>,
    ) {
        let origin = self.treemap_origin.get();
        let x = (event.position.x - origin.x).as_f32();
        let y = (event.position.y - origin.y).as_f32();
        let crumbs = self.tile_at(x, y);

        match event.button {
            MouseButton::Left if event.click_count >= 2 => {
                if let Some(crumbs) = crumbs {
                    self.select(Some(crumbs), cx);
                    self.descend(cx);
                }
            }
            MouseButton::Left
                if event.modifiers.control || event.modifiers.platform =>
            {
                if let Some(crumbs) = crumbs {
                    self.toggle_mark(&crumbs, cx);
                }
            }
            MouseButton::Left => {
                let activate = crumbs.as_ref().is_some_and(|crumbs| {
                    self.selected.as_ref() == Some(crumbs)
                        && self.node_at(crumbs).is_some_and(Node::is_dir)
                });
                if activate {
                    // A second click on the selection opens it, like a file
                    // manager, without needing a double click.
                    self.descend(cx);
                    return;
                }
                self.select(crumbs, cx);
            }
            MouseButton::Middle => {
                if let Some(crumbs) = crumbs {
                    self.toggle_mark(&crumbs, cx);
                }
            }
            _ => {}
        }
    }

    pub fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        cx: &mut Context<'_, Self>,
    ) {
        let origin = self.treemap_origin.get();
        let x = (event.position.x - origin.x).as_f32();
        let y = (event.position.y - origin.y).as_f32();
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => delta.y,
            ScrollDelta::Pixels(delta) => delta.y.as_f32() / 24.0,
        };
        if lines.abs() < f32::EPSILON {
            return;
        }
        if event.modifiers.shift {
            // Pan instead of zoom, for looking around a magnified view.
            self.view.origin_y =
                (self.view.origin_y - lines * 40.0 / self.view.scale).max(0.0);
            cx.notify();
            return;
        }
        let factor = if lines > 0.0 { 1.15 } else { 1.0 / 1.15 };
        self.zoom_at(x, y, factor, true, cx);
    }

    /// Advance the layout transition, if one is running.
    pub fn tick_transition(&mut self, window: &Window) {
        if let Some(transition) = self.transition {
            let (_, running) = transition.sample(Rect::default());
            if running {
                window.request_animation_frame();
            } else {
                self.transition = None;
            }
        }
    }

    /// A tile's rectangle for this frame: mid-transition it is on its way from
    /// where it was to where it is.
    fn animated_rect(&self, rect: Rect) -> Rect {
        match self.transition {
            Some(transition) => transition.sample(rect).0,
            None => rect,
        }
    }
}

/// A direction for geometric selection movement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    const fn is_backwards(self) -> bool {
        matches!(self, Self::Left | Self::Up)
    }

    /// Distance from `from` to `rect` along the axis, if `rect` lies that way.
    fn gap(self, from: &Rect, rect: &Rect) -> Option<f32> {
        let epsilon = 0.5;
        match self {
            Self::Right => {
                let gap = rect.x - from.right();
                (gap >= -epsilon).then_some(gap.max(0.0))
            }
            Self::Left => {
                let gap = from.x - rect.right();
                (gap >= -epsilon).then_some(gap.max(0.0))
            }
            Self::Down => {
                let gap = rect.y - from.bottom();
                (gap >= -epsilon).then_some(gap.max(0.0))
            }
            Self::Up => {
                let gap = from.y - rect.bottom();
                (gap >= -epsilon).then_some(gap.max(0.0))
            }
        }
    }

    /// How far off the direction's axis `rect` sits, so the nearest tile in the
    /// direction wins rather than any tile in that half-plane.
    fn offset(self, from: &Rect, rect: &Rect) -> f32 {
        let overlap =
            |a_start: f32, a_end: f32, b_start: f32, b_end: f32| -> f32 {
                (a_end.min(b_end) - a_start.max(b_start)).max(0.0)
            };
        match self {
            Self::Left | Self::Right => {
                let shared =
                    overlap(from.y, from.bottom(), rect.y, rect.bottom());
                (from.h.min(rect.h) - shared).max(0.0)
            }
            Self::Up | Self::Down => {
                let shared =
                    overlap(from.x, from.right(), rect.x, rect.right());
                (from.w.min(rect.w) - shared).max(0.0)
            }
        }
    }
}

/// A tile's label, resolved for painting.
#[derive(Clone, Debug)]
pub struct Label {
    pub text: String,
    /// Base-space rectangle; the view transform is applied while painting.
    pub rect: Rect,
    /// The band this label belongs in, when its tile reserved one. A parent's
    /// name goes in its own band, never over its children.
    pub header: Option<Rect>,
    /// Depth from the scanned root, used for the colour so it does not shift
    /// when the view descends.
    pub color_depth: u32,
    pub marked: bool,
    pub size_text: String,
}

/// How many labels one frame will shape.
const MAX_LABELS: usize = 150;

/// Height of a directory's name band, in rem.
const HEADER_REMS: f32 = 1.0625;

/// Smallest tile, in rem, that gets a label: below this a name cannot be read.
const LABEL_MIN_W_REMS: f32 = 3.375;
const LABEL_MIN_H_REMS: f32 = 0.9375;

/// Where keyboard focus should go once a window is available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusTarget {
    /// The treemap and screens, which own every key binding.
    Root,
    /// The permanent-deletion alert dialog.
    Dialog,
}

impl Render for Disktree {
    fn render(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> impl gpui_kit::IntoElement {
        self.rem = window.rem_size().as_f32();
        self.tick_transition(window);
        crate::views::root(self, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> View {
        View {
            scale: 2.0,
            origin_x: 100.0,
            origin_y: 50.0,
        }
    }

    #[test]
    fn projecting_and_unprojecting_are_inverses() {
        let view = view();
        let (x, y) = view.unproject(400.0, 300.0);
        let rect = Rect::new(x, y, 10.0, 10.0);
        let screen = view.project(rect);
        assert!((screen.x - 400.0).abs() < 0.001);
        assert!((screen.y - 300.0).abs() < 0.001);
        assert!((screen.w - 20.0).abs() < 0.001);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_cursor_still() {
        let view = View::IDENTITY;
        let (before_x, before_y) = view.unproject(300.0, 200.0);
        let zoomed = view.zoomed_at(300.0, 200.0, 2.0, View::MAX_SCALE);
        let rect = Rect::new(before_x, before_y, 1.0, 1.0);
        let screen = zoomed.project(rect);
        assert!((screen.x - 300.0).abs() < 0.01, "{screen:?}");
        assert!((screen.y - 200.0).abs() < 0.01, "{screen:?}");
    }

    #[test]
    fn clamping_never_leaves_a_margin() {
        let area = size(px(800.), px(600.));
        let clamped = View {
            scale: 2.0,
            origin_x: -500.0,
            origin_y: 900.0,
        }
        .clamped(area);
        assert!(clamped.origin_x.abs() < f32::EPSILON);
        assert!((clamped.origin_y - 300.0).abs() < f32::EPSILON);

        let identity = View {
            scale: 1.0,
            origin_x: 40.0,
            origin_y: 40.0,
        }
        .clamped(area);
        assert!(identity.origin_x.abs() < f32::EPSILON);
        assert!(identity.origin_y.abs() < f32::EPSILON);
    }

    #[test]
    fn a_transition_starts_where_the_region_was_and_ends_where_it_is() {
        // A region that occupied the whole viewport, landing in the top-left
        // quarter of the new layout: everything inside it must start twice as
        // large and centred where it was.
        let transition = LayoutTransition::new(
            Rect::new(0.0, 0.0, 800.0, 600.0),
            Rect::new(0.0, 0.0, 400.0, 300.0),
        );
        // A tile inside the destination, at the far corner.
        let tile = Rect::new(200.0, 150.0, 200.0, 150.0);
        let (_, running) = transition.sample(tile);
        assert!(running, "the transition is in flight");
        let origin = transition.origin_of(tile);
        assert!((origin.x - 400.0).abs() < 0.001, "{origin:?}");
        assert!((origin.y - 300.0).abs() < 0.001, "{origin:?}");
        assert!((origin.w - 400.0).abs() < 0.001, "{origin:?}");
        assert!((origin.h - 300.0).abs() < 0.001, "{origin:?}");

        let finished = LayoutTransition {
            started: Instant::now()
                .checked_sub(Duration::from_millis(500))
                .expect("a moment on a running machine"),
            ..transition
        };
        let (end, running) = finished.sample(tile);
        assert!(!running);
        assert_eq!(end, tile, "it ends exactly at the real layout");
    }

    #[test]
    fn a_transition_grows_a_tile_that_is_being_entered() {
        // Entering a child: the child's region becomes the viewport, so
        // everything inside it gets bigger, never smaller.
        let transition = LayoutTransition::new(
            Rect::new(100.0, 100.0, 200.0, 200.0),
            Rect::new(0.0, 0.0, 800.0, 600.0),
        );
        let tile = Rect::new(400.0, 300.0, 100.0, 100.0);
        let origin = transition.origin_of(tile);
        assert!(origin.w < tile.w, "it starts smaller: {origin:?}");
        assert!(
            origin.x > 100.0 && origin.x < 300.0,
            "and where the child was: {origin:?}"
        );
    }

    #[test]
    fn directions_only_see_gaps_on_their_own_side() {
        let from = Rect::new(100.0, 100.0, 50.0, 50.0);
        let right = Rect::new(200.0, 100.0, 50.0, 50.0);
        let left = Rect::new(0.0, 100.0, 50.0, 50.0);
        assert!(Direction::Right.gap(&from, &right).is_some());
        assert!(Direction::Right.gap(&from, &left).is_none());
        assert!(Direction::Left.gap(&from, &left).is_some());
        assert!(Direction::Down.gap(&from, &right).is_none());
        assert_eq!(Direction::Right.gap(&from, &right), Some(50.0));
        // Aligned neighbours have no perpendicular offset; stacked ones do.
        assert!(Direction::Right.offset(&from, &right).abs() < f32::EPSILON);
        assert!(
            Direction::Right
                .offset(&from, &Rect::new(200.0, 160.0, 20.0, 20.0))
                > 0.0
        );
    }
}

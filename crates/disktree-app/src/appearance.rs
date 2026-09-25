//! Light or dark when there is no Omarchy theme to follow.
//!
//! On Omarchy, gpui-omarchy follows the desktop's theme files. A Mac has none,
//! and gpui-omarchy would then stay on its dark default whatever the system
//! is set to; there the app follows the system appearance instead, with the
//! crate's own dark and light palettes, and switches when it does.

use std::path::Path;

use gpui_kit::{App, Window, WindowAppearance};
use gpui_omarchy::Theme;

/// Whether the system appearance, rather than an Omarchy theme, decides the
/// colours: on macOS, unless an Omarchy theme is installed there anyway.
pub fn follows_system(home: Option<&Path>) -> bool {
    cfg!(target_os = "macos")
        && !home.is_some_and(|home| {
            [".local/state/omarchy/current", ".config/omarchy/current"]
                .iter()
                .any(|theme| home.join(theme).exists())
        })
}

/// Apply the palette for `appearance`. Applying an explicit theme also stops
/// gpui-omarchy watching for theme files that are not there.
pub fn apply(appearance: WindowAppearance, cx: &mut App) {
    let theme = match appearance {
        WindowAppearance::Light | WindowAppearance::VibrantLight => {
            Theme::flexoki_light()
        }
        WindowAppearance::Dark | WindowAppearance::VibrantDark => {
            Theme::tokyo_night()
        }
    };
    theme.apply(cx);
}

/// Follow `window`'s appearance from now on.
pub fn follow(window: &Window) {
    window
        .observe_window_appearance(|window, cx| apply(window.appearance(), cx))
        .detach();
}

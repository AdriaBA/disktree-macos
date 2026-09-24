//! Colours derived from the Omarchy theme.
//!
//! Nothing here invents a palette: every colour is the active theme's, shifted
//! in hue, lightness or alpha. That keeps the app inside Omarchy's visual
//! language across themes, and light and dark alike.

use gpui_kit::Hsla;
use gpui_kit::base::ThemeAppearance;
use gpui_omarchy::Theme;

/// Hue for a tile: nesting walks the wheel so levels are readable without
/// borders, and hidden entries get a hue of their own instead of a shade.
///
/// Hidden directories under a home directory are usually the largest thing
/// there — `~/.cache`, `~/.cargo`, `~/.local/share/Trash` — so they are marked
/// by colour rather than hidden away.
pub fn tile_hue(theme: &Theme, depth: u32, hidden: bool) -> f32 {
    let base = if hidden {
        theme.warning.h
    } else {
        theme.accent.h
    };
    wrap_hue((depth as f32).mul_add(0.075, base))
}

fn wrap_hue(hue: f32) -> f32 {
    let hue = hue % 1.0;
    if hue < 0.0 { hue + 1.0 } else { hue }
}

/// Fill for a tile at `depth`, as an opaque colour the mosaic can paint.
pub fn tile_fill(
    theme: &Theme,
    depth: u32,
    hidden: bool,
    marked: bool,
) -> Hsla {
    let dark = theme.appearance == ThemeAppearance::Dark;
    // Deeper levels lift slightly so nested tiles separate from their parent
    // without needing a border on every one of them.
    let lightness = if dark {
        (depth.min(4) as f32).mul_add(0.055, 0.20)
    } else {
        (depth.min(4) as f32).mul_add(-0.055, 0.86)
    };
    let saturation = if hidden { 0.55 } else { 0.42 };
    let fill = Hsla {
        h: tile_hue(theme, depth, hidden),
        s: saturation,
        l: lightness,
        a: 1.0,
    };
    // Keep the mosaic calm: pull it toward the theme background so the tile
    // colours read as tinted surfaces rather than a rainbow.
    let flattened = mix(fill, theme.background, 0.34);
    if marked {
        mix(flattened, theme.danger, 0.30)
    } else {
        flattened
    }
}

/// Linear interpolation between two colours in HSLA.
pub fn mix(from: Hsla, to: Hsla, t: f32) -> Hsla {
    let t = t.clamp(0.0, 1.0);
    let lerp = |a: f32, b: f32| (b - a).mul_add(t, a);
    Hsla {
        h: lerp(from.h, to.h),
        s: lerp(from.s, to.s),
        l: lerp(from.l, to.l),
        a: lerp(from.a, to.a),
    }
}

/// The colour of a tile's label on top of its fill.
pub fn label_color(theme: &Theme, depth: u32) -> Hsla {
    let dark = theme.appearance == ThemeAppearance::Dark;
    let base = if dark { theme.bright } else { theme.background };
    if depth == 0 { base } else { base.opacity(0.86) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(appearance: ThemeAppearance) -> Theme {
        let mut theme = Theme::tokyo_night();
        theme.appearance = appearance;
        theme
    }

    #[test]
    fn hidden_tiles_are_a_different_hue_from_visible_ones() {
        let theme = theme(ThemeAppearance::Dark);
        let visible = tile_fill(&theme, 0, false, false);
        let hidden = tile_fill(&theme, 0, true, false);
        assert!((visible.h - hidden.h).abs() > 0.02);
        assert!(visible.s > 0.0);
    }

    #[test]
    fn marking_shifts_a_tile_toward_the_danger_colour() {
        let theme = theme(ThemeAppearance::Dark);
        let plain = tile_fill(&theme, 1, false, false);
        let marked = tile_fill(&theme, 1, false, true);
        let plain_distance = (plain.h - theme.danger.h).abs();
        let marked_distance = (marked.h - theme.danger.h).abs();
        assert!(marked_distance < plain_distance);
    }

    #[test]
    fn light_and_dark_appearances_differ_in_lightness() {
        let dark = tile_fill(&theme(ThemeAppearance::Dark), 0, false, false);
        let light = tile_fill(&theme(ThemeAppearance::Light), 0, false, false);
        assert!(light.l > dark.l);
    }

    #[test]
    fn hue_stays_in_range_at_every_depth() {
        let theme = theme(ThemeAppearance::Dark);
        for depth in 0..12 {
            let hue = tile_hue(&theme, depth, false);
            assert!((0.0..1.0).contains(&hue), "depth {depth} gave {hue}");
        }
    }

    #[test]
    fn mix_clamps_its_parameter() {
        let theme = theme(ThemeAppearance::Dark);
        let clamped_low = mix(theme.background, theme.accent, -1.0).l;
        let clamped_high = mix(theme.background, theme.accent, 2.0).l;
        assert!((clamped_low - theme.background.l).abs() < f32::EPSILON);
        assert!((clamped_high - theme.accent.l).abs() < f32::EPSILON);
    }
}

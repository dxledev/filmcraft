//! Shell theme: tint the dark themes with the colours of the desktop shell's active theme.
//!
//! The host (`apps/filmcraft/src/shell_theme.rs`) reads the palette the shell applies, whether it
//! is a wallpaper-generated dynamic palette or a static theme, and hands it over through
//! [`crate::HostHooks::shell_palette`]. This module holds the part that needs no file access:
//! reading a palette file, tinting [`Tokens`] and keeping the result readable.
//!
//! Tinting changes hue and chroma only. Every neutral token keeps its OKLab lightness, so the
//! hierarchy of the stock dark theme (panel, field, hover, text) and its contrast survive any
//! palette. Semantic colours (render bars, danger, caption tracks) are left alone, the accent
//! family follows the shell's primary colour, and a final pass lifts text, icons and accents to
//! WCAG ratios against the surfaces they are drawn on.

use egui::Color32;

use crate::theme::{ThemeKind, Tokens};

/// The most a palette file may weigh. Shell palettes are well under 4 KiB.
pub const MAX_PALETTE_BYTES: usize = 1 << 20;

/// Which of the shell's palettes to follow (Settings ▸ Appearance ▸ Shell Theme).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShellSource {
    /// Whatever the shell applies to applications right now, dynamic or static.
    #[default]
    Auto,
    /// The palette generated from the wallpaper.
    Dynamic,
    /// The theme chosen by name.
    Static,
}

impl ShellSource {
    /// The source a `appearance.shellTheme` value names; `None` for `off` and unknown values.
    pub fn from_pref(s: &str) -> Option<ShellSource> {
        match s {
            "auto" => Some(ShellSource::Auto),
            "dynamic" => Some(ShellSource::Dynamic),
            "static" => Some(ShellSource::Static),
            _ => None,
        }
    }
}

/// The three colours of a shell theme that matter here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellPalette {
    pub background: [u8; 3],
    pub foreground: [u8; 3],
    pub primary: [u8; 3],
}

// ---------------------------------------------------------------------------------------------
// Reading palette files
// ---------------------------------------------------------------------------------------------

/// Read a palette from the text of a shell theme file: a Quickshell `Colors.qml`
/// (`property color background: "#141520"`), or JSON with a `colours` / `colors` object (hex with
/// or without `#`), a Noctalia `dark` object, or the colours at the top level. `None` when the text
/// is not a palette, is too large, or lacks a background or a primary colour.
pub fn parse_palette(text: &str) -> Option<ShellPalette> {
    if text.len() > MAX_PALETTE_BYTES {
        return None;
    }
    let colors = if text.trim_start().starts_with('{') { json_colors(text)? } else { qml_colors(text) };
    let pick = |keys: &[&str]| keys.iter().find_map(|k| colors.iter().find(|(name, _)| name == k).map(|(_, c)| *c));
    let background = pick(&["background", "base", "m3background", "mSurface", "surface"])?;
    let primary = pick(&["primary", "accent", "mPrimary", "m3primary"])?;
    let foreground = pick(&["foreground", "onBackground", "on_background", "text", "mOnSurface", "onSurface", "on_surface"])
        .unwrap_or_else(|| mix(background, [255, 255, 255], 0.88));
    Some(ShellPalette { background, foreground, primary })
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let f = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round().clamp(0.0, 255.0) as u8;
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]
}

/// `#rrggbb`, `rrggbb`, or either with a trailing alpha pair (ignored).
fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if !(s.len() == 6 || s.len() == 8) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
    Some([byte(0)?, byte(2)?, byte(4)?])
}

fn json_colors(text: &str) -> Option<Vec<(String, [u8; 3])>> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    let object = ["colours", "colors", "dark"].iter().find_map(|k| root.get(k).and_then(|v| v.as_object())).or_else(|| root.as_object())?;
    Some(object.iter().filter_map(|(k, v)| Some((k.clone(), parse_hex(v.as_str()?)?))).collect())
}

fn qml_colors(text: &str) -> Vec<(String, [u8; 3])> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("");
        let Some(rest) = line.split("property color").nth(1) else { continue };
        let Some((name, value)) = rest.split_once(':') else { continue };
        let name = name.trim();
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            continue;
        }
        let value = value.trim();
        let color = if let Some(quoted) = value.strip_prefix(['"', '\'']) {
            quoted.split(['"', '\'']).next().and_then(parse_hex)
        } else if let Some(args) = value.strip_prefix("Qt.rgba(") {
            qt_rgba(args.split(')').next().unwrap_or(""))
        } else {
            None
        };
        if let Some(c) = color {
            out.push((name.to_string(), c));
        }
    }
    out
}

/// `Qt.rgba(38 / 255, 36 / 255, 45 / 255, 0.87)`: channels in 0..=1, each optionally `a / b`.
fn qt_rgba(args: &str) -> Option<[u8; 3]> {
    let channel = |expr: &str| -> Option<f32> {
        let mut parts = expr.split('/');
        let numerator: f32 = parts.next()?.trim().parse().ok()?;
        let value = match parts.next() {
            Some(d) => {
                let d: f32 = d.trim().parse().ok()?;
                if d <= 0.0 {
                    return None;
                }
                numerator / d
            }
            None => numerator,
        };
        (parts.next().is_none() && value.is_finite() && (0.0..=1.0).contains(&value)).then_some(value)
    };
    let mut it = args.split(',');
    let (r, g, b) = (channel(it.next()?)?, channel(it.next()?)?, channel(it.next()?)?);
    let byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    Some([byte(r), byte(g), byte(b)])
}

// ---------------------------------------------------------------------------------------------
// Colour maths: OKLab for tinting, WCAG for contrast
// ---------------------------------------------------------------------------------------------

fn to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn from_linear(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// WCAG relative luminance.
fn luminance(c: [u8; 3]) -> f32 {
    0.2126 * to_linear(c[0]) + 0.7152 * to_linear(c[1]) + 0.0722 * to_linear(c[2])
}

/// WCAG contrast ratio, 1.0 to 21.0.
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// OKLCH: lightness, chroma, hue in radians.
#[derive(Clone, Copy, Debug)]
struct Lch {
    l: f32,
    c: f32,
    h: f32,
}

fn to_lch(rgb: [u8; 3]) -> Lch {
    let (r, g, b) = (to_linear(rgb[0]), to_linear(rgb[1]), to_linear(rgb[2]));
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    let ok_l = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let bb = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    Lch { l: ok_l, c: a.hypot(bb), h: bb.atan2(a) }
}

/// The sRGB colour of `lch`, or `None` when it falls outside the gamut.
fn lch_in_gamut(lch: Lch) -> Option<[u8; 3]> {
    let (a, b) = (lch.c * lch.h.cos(), lch.c * lch.h.sin());
    let l = (lch.l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (lch.l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (lch.l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    let r = 4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s;
    let g = -1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s;
    let bl = -0.004_196_086 * l - 0.703_418_6 * m + 1.707_614_7 * s;
    let ok = |v: f32| (-0.002..=1.002).contains(&v);
    (ok(r) && ok(g) && ok(bl)).then(|| {
        let byte = |v: f32| (from_linear(v) * 255.0).round().clamp(0.0, 255.0) as u8;
        [byte(r), byte(g), byte(bl)]
    })
}

/// `lch` as sRGB, reducing its chroma until it fits the gamut.
fn from_lch(lch: Lch) -> [u8; 3] {
    let l = if lch.l.is_finite() { lch.l.clamp(0.0, 1.0) } else { 0.0 };
    let (mut lo, mut hi) = (0.0_f32, if lch.c.is_finite() { lch.c.max(0.0) } else { 0.0 });
    if let Some(rgb) = lch_in_gamut(Lch { l, c: hi, h: lch.h }) {
        return rgb;
    }
    for _ in 0..14 {
        let mid = (lo + hi) / 2.0;
        if lch_in_gamut(Lch { l, c: mid, h: lch.h }).is_some() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lch_in_gamut(Lch { l, c: lo, h: lch.h }).unwrap_or([0, 0, 0])
}

/// Move `fg`'s lightness (hue and chroma kept) until it reaches `min` contrast against every
/// background: lighter when `lighten`, darker otherwise. Returns `fg` unchanged if it already does.
fn ensure_contrast(fg: [u8; 3], backgrounds: &[[u8; 3]], min: f32, lighten: bool) -> [u8; 3] {
    let worst = |c: [u8; 3]| backgrounds.iter().map(|b| contrast_ratio(c, *b)).fold(f32::INFINITY, f32::min);
    if worst(fg) >= min {
        return fg;
    }
    let lch = to_lch(fg);
    let mut best = fg;
    for step in 1..=50 {
        let l = if lighten { lch.l + step as f32 * 0.02 } else { lch.l - step as f32 * 0.02 };
        best = from_lch(Lch { l, ..lch });
        if worst(best) >= min || !(0.0..=1.0).contains(&l) {
            break;
        }
    }
    best
}

// ---------------------------------------------------------------------------------------------
// Tinting
// ---------------------------------------------------------------------------------------------

/// Chroma at or below which a token counts as a grey that takes a tint.
const NEUTRAL_CHROMA: f32 = 0.02;
/// Surface tint strength, as OKLab chroma: enough to read as the palette, quiet enough to stay a
/// neutral dark.
const SURFACE_CHROMA: std::ops::RangeInclusive<f32> = 0.012..=0.040;
/// Lightness above which a neutral is text or an icon and takes the foreground's hue.
const TEXT_LIGHTNESS: f32 = 0.62;

struct Tint {
    surface_hue: f32,
    surface_chroma: f32,
    text_hue: f32,
    text_chroma: f32,
}

impl Tint {
    fn new(p: &ShellPalette) -> Tint {
        let (bg, fg, primary) = (to_lch(p.background), to_lch(p.foreground), to_lch(p.primary));
        // A grey background has no hue: borrow the primary's, at the floor strength.
        let (surface_hue, surface_chroma) =
            if bg.c < 0.008 { (primary.h, *SURFACE_CHROMA.start()) } else { (bg.h, bg.c.clamp(*SURFACE_CHROMA.start(), *SURFACE_CHROMA.end())) };
        let (text_hue, text_chroma) = if fg.c < 0.006 { (surface_hue, 0.008) } else { (fg.h, fg.c.min(0.03)) };
        Tint { surface_hue, surface_chroma, text_hue, text_chroma }
    }

    fn apply(&self, c: &mut Color32) {
        if c.a() != 255 {
            return;
        }
        let lch = to_lch([c.r(), c.g(), c.b()]);
        if lch.c > NEUTRAL_CHROMA || lch.l < 0.03 {
            return;
        }
        let (h, chroma) = if lch.l >= TEXT_LIGHTNESS { (self.text_hue, self.text_chroma) } else { (self.surface_hue, self.surface_chroma) };
        let [r, g, b] = from_lch(Lch { l: lch.l, c: chroma, h });
        *c = Color32::from_rgb(r, g, b);
    }
}

fn rgb(c: Color32) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

fn color(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// `tokens` of a dark theme, tinted with `palette`. With `shell_accent`, the accent family
/// (selection, primary buttons, focus, playhead, timecodes, track targeting) follows the
/// palette's primary colour; otherwise the highlight colour in `tokens` stays.
pub fn tint_tokens(mut t: Tokens, palette: &ShellPalette, shell_accent: bool) -> Tokens {
    if t.kind == ThemeKind::Light {
        return t;
    }
    let tint = Tint::new(palette);
    // The pitch-black gutter between panels cannot carry a tint: lift it to a very dark surface.
    if t.app_bg == Color32::BLACK {
        t.app_bg = color(from_lch(Lch { l: 0.1, c: tint.surface_chroma, h: tint.surface_hue }));
    }
    macro_rules! tint_all {
        ($($f:ident),* $(,)?) => { $( tint.apply(&mut t.$f); )* };
    }
    tint_all!(
        app_bg,
        header_bg,
        panel_bg,
        tab_bg,
        tab_text,
        tab_text_active,
        text,
        text_dim,
        text_faint,
        icon,
        icon_active,
        hover,
        pressed,
        field_bg,
        field_border,
        separator,
        row_alt,
        row_selected,
        pill_active_bg,
        clip_bar_bg,
        control_surface,
        control_well,
        control_border,
        control_handle_bg,
        control_handle,
        control_handle_dim,
        plot_bg,
        plot_grid,
        plot_axis,
        plot_handle,
        plot_handle_dim,
        meter_readout_bg,
        meter_clip_off,
        panner_bg,
        panner_border,
        panner_speaker,
        fader_knob,
        eq_track,
        eq_tick,
        eq_knob,
        eq_knob_active,
        eq_node,
        eq_node_off,
        crossover_handle,
        keyframe_plot_bg,
        keyframe_plot_grid,
        keyframe_plot_axis,
        keyframe_handle,
        switch_fill,
        switch_knob,
        switch_border,
        slider_disabled_track,
        slider_disabled_knob,
        button_hover,
        button_border,
        scroll_thumb,
        scroll_thumb_hover,
        vertical_scroll_thumb,
        disabled_clip_bg,
        tl_bg,
        tl_track_bg,
        tl_track_bg_alt,
        tl_header_bg,
        tl_ruler_bg,
        tl_ruler_tick,
        tl_ruler_text,
        in_out_shade,
        clip_selected_border,
        monitor_bg,
        bin_tile_bg,
    );

    let surfaces: Vec<[u8; 3]> = [
        t.panel_bg,
        t.tab_bg,
        t.header_bg,
        t.field_bg,
        t.tl_bg,
        t.tl_track_bg,
        t.tl_header_bg,
        t.tl_ruler_bg,
        t.monitor_bg,
        t.hover,
        t.row_selected,
        t.row_alt,
        t.pill_active_bg,
        t.button_hover,
        t.control_surface,
    ]
    .into_iter()
    .map(rgb)
    .collect();

    // Text-coloured accents sit on the surfaces; filled accents carry white text.
    let light = ensure_contrast(palette.primary, &surfaces, 4.5, true);
    // Bins take the shell's primary whatever the highlight colour is.
    t.bin_color = color(light);
    if shell_accent {
        let primary = palette.primary;
        let fill = ensure_contrast(primary, &[[255, 255, 255]], 4.8, false);
        let lch = to_lch(fill);
        let hover = ensure_contrast(from_lch(Lch { l: lch.l + 0.04, ..lch }), &[[255, 255, 255]], 4.5, false);
        let target = ensure_contrast(from_lch(Lch { l: lch.l - 0.05, ..lch }), &[[255, 255, 255]], 4.8, false);
        t.accent = color(fill);
        t.accent_hover = color(hover);
        t.focus = color(light);
        t.playhead = color(light);
        t.timecode = color(light);
        t.hot_text = color(light);
        t.track_target = color(target);
    }

    // Readable text whatever the palette: body text 7:1, secondary text and icons 4.5:1, hints 3:1.
    for (field, min) in [
        (&mut t.text, 7.0),
        (&mut t.tab_text_active, 7.0),
        (&mut t.icon_active, 7.0),
        (&mut t.text_dim, 4.5),
        (&mut t.tab_text, 4.5),
        (&mut t.icon, 4.5),
        (&mut t.tl_ruler_text, 4.5),
        (&mut t.text_faint, 3.0),
        (&mut t.tl_ruler_tick, 3.0),
    ] {
        *field = color(ensure_contrast(rgb(*field), &surfaces, min, true));
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISLAND: &str = r##"pragma Singleton
import Quickshell
import QtQuick

Singleton {
    readonly property color background: "#141520"
    readonly property color backgroundAlt: "#10111a"
    readonly property color backgroundAlpha: Qt.rgba(38 / 255, 36 / 255, 45 / 255, 0.87)
    readonly property color foreground: "#e2c9b4"
    readonly property color primary: "#9badd9"
}
"##;

    fn palette(bg: [u8; 3], fg: [u8; 3], primary: [u8; 3]) -> ShellPalette {
        ShellPalette { background: bg, foreground: fg, primary }
    }

    #[test]
    fn reads_quickshell_colors_qml() {
        let p = parse_palette(ISLAND).unwrap();
        assert_eq!(p, palette([0x14, 0x15, 0x20], [0xe2, 0xc9, 0xb4], [0x9b, 0xad, 0xd9]));
    }

    #[test]
    fn reads_json_palettes_with_and_without_hash() {
        let caelestia = r#"{"colours":{"background":"0a191c","foreground":"ddcbb1","accent":"8eb1dd"}}"#;
        assert_eq!(parse_palette(caelestia), Some(palette([0x0a, 0x19, 0x1c], [0xdd, 0xcb, 0xb1], [0x8e, 0xb1, 0xdd])));
        let noctalia = r##"{"dark":{"mSurface":"#0f0f0f","mOnSurface":"#e0d8d0","mPrimary":"#e66a4dff"}}"##;
        assert_eq!(parse_palette(noctalia), Some(palette([0x0f; 3], [0xe0, 0xd8, 0xd0], [0xe6, 0x6a, 0x4d])));
        let flat = r#"{"base":"141520","text":"e2c9b4","primary":"9badd9"}"#;
        assert!(parse_palette(flat).is_some());
    }

    #[test]
    fn hostile_palettes_are_rejected_not_fatal() {
        for text in [
            "",
            "{",
            "[]",
            "null",
            "pragma Singleton",
            r#"{"colours":{"background":"zzzzzz","primary":"123"}}"#,
            r##"{"colours":{"background":"#12345","primary":"#1234567"}}"##,
            "property color background: Qt.rgba(1 / 0, 0, 0, 1)\nproperty color primary: \"#ffffff\"",
            "property color background: Qt.rgba(2, 0, 0, 1)\nproperty color primary: \"#ffffff\"",
            "property color background: Qt.rgba(nan, 0, 0, 1)\nproperty color primary: \"#ffffff\"",
            "property color background:",
            "property color : \"#ffffff\"",
            "property color primary: \"#ffffff\"",
            "property color background: \"#é1é1é1\"\nproperty color primary: \"#ffffff\"",
        ] {
            assert_eq!(parse_palette(text), None, "{text:?}");
        }
        assert_eq!(parse_palette(&"x".repeat(MAX_PALETTE_BYTES + 1)), None);
        // a missing foreground is derived, not an error
        let p = parse_palette("property color background: \"#101010\"\nproperty color primary: \"#ff8800\"").unwrap();
        assert!(contrast_ratio(p.foreground, p.background) > 7.0);
    }

    #[test]
    fn oklch_round_trips_and_stays_in_gamut() {
        for c in [[0, 0, 0], [255, 255, 255], [0x14, 0x15, 0x20], [0xe6, 0x6a, 0x4d], [0x2f, 0x6b, 0xdf], [12, 200, 90]] {
            let back = from_lch(to_lch(c));
            for i in 0..3 {
                assert!((i32::from(back[i]) - i32::from(c[i])).abs() <= 1, "{c:?} -> {back:?}");
            }
        }
        // out-of-gamut asks lose chroma instead of wrapping
        let c = from_lch(Lch { l: 0.9, c: 0.4, h: 2.0 });
        assert!(luminance(c) > 0.5);
        assert_eq!(from_lch(Lch { l: f32::NAN, c: f32::NAN, h: f32::NAN }), [0, 0, 0]);
    }

    #[test]
    fn tinting_keeps_lightness_and_adopts_the_palette_hue() {
        let stock = Tokens::for_kind(ThemeKind::Dark);
        let island = palette([0x14, 0x15, 0x20], [0xe2, 0xc9, 0xb4], [0x9b, 0xad, 0xd9]);
        let t = tint_tokens(stock, &island, true);
        for (a, b) in [(stock.panel_bg, t.panel_bg), (stock.field_bg, t.field_bg), (stock.hover, t.hover), (stock.separator, t.separator)] {
            let (la, lb) = (to_lch(rgb(a)).l, to_lch(rgb(b)).l);
            assert!((la - lb).abs() < 0.01, "{a:?} {b:?}");
        }
        // the panel now leans the way the shell's background does: blue over red
        assert!(t.panel_bg.b() > t.panel_bg.r());
        // the gutter is no longer pitch black, but is still darker than the panels
        assert!(t.app_bg != Color32::BLACK && luminance(rgb(t.app_bg)) < luminance(rgb(t.panel_bg)));
        // bins follow the palette, on a tile that is tinted too
        assert_ne!(t.bin_color, stock.bin_color);
        assert!(t.bin_tile_bg.b() > t.bin_tile_bg.r(), "{:?}", t.bin_tile_bg);
        // semantic colours are untouched
        assert_eq!((t.render_red, t.danger, t.caption_track_bg, t.meter_bg), (stock.render_red, stock.danger, stock.caption_track_bg, stock.meter_bg));
        // a theme without a palette is exactly as designed, and the light theme is never tinted
        let light = Tokens::for_kind(ThemeKind::Light);
        assert_eq!(tint_tokens(light, &island, true), light);
    }

    #[test]
    fn a_grey_palette_borrows_the_primary_hue() {
        // Firesky: a grey #0f0f0f background with an orange primary.
        let firesky = palette([0x0f; 3], [0xe0, 0xd8, 0xd0], [0xe6, 0x6a, 0x4d]);
        let t = tint_tokens(Tokens::for_kind(ThemeKind::Dark), &firesky, true);
        assert!(t.panel_bg.r() > t.panel_bg.b(), "{:?}", t.panel_bg);
    }

    #[test]
    fn a_users_own_highlight_survives_the_tint() {
        let stock = Tokens::for_kind(ThemeKind::Dark);
        let t = tint_tokens(stock, &palette([0x14, 0x15, 0x20], [0xe2, 0xc9, 0xb4], [0xe6, 0x6a, 0x4d]), false);
        assert_eq!((t.accent, t.focus, t.playhead, t.track_target), (stock.accent, stock.focus, stock.playhead, stock.track_target));
    }

    /// Whatever a shell throws at it, text is readable and accents carry white text and show up on
    /// the surfaces: the guarantee that "match my theme" never costs legibility.
    #[test]
    fn every_palette_keeps_text_and_accents_readable() {
        let palettes = [
            palette([0x14, 0x15, 0x20], [0xe2, 0xc9, 0xb4], [0x9b, 0xad, 0xd9]),
            palette([0x18, 0x12, 0x15], [0xed, 0xdf, 0xe4], [0xf7, 0xb2, 0xdf]),
            palette([0x0f; 3], [0xe0, 0xd8, 0xd0], [0xe6, 0x6a, 0x4d]),
            // a light "dark" palette and a text colour darker than its background
            palette([0xf5, 0xf0, 0xe8], [0x20, 0x20, 0x20], [0x1a, 0x3a, 0x8f]),
            palette([0x00, 0x00, 0x00], [0x00, 0x00, 0x00], [0x00, 0x00, 0x00]),
            palette([0xff, 0xff, 0xff], [0xff, 0xff, 0xff], [0xff, 0xff, 0xff]),
            // saturated, and a primary too dark or too light for either role
            palette([0x00, 0x30, 0xff], [0xff, 0x00, 0x00], [0x00, 0xff, 0x00]),
            palette([0x10, 0x10, 0x30], [0xee, 0xee, 0xee], [0x10, 0x10, 0x60]),
            palette([0x10, 0x30, 0x10], [0xee, 0xee, 0xee], [0xff, 0xff, 0xe0]),
        ];
        for kind in [ThemeKind::Dark, ThemeKind::Medium] {
            for p in &palettes {
                let t = tint_tokens(Tokens::for_kind(kind), p, true);
                let surfaces = [t.panel_bg, t.tab_bg, t.field_bg, t.tl_bg, t.hover, t.row_selected, t.pill_active_bg];
                let ratio = |fg: Color32| surfaces.iter().map(|s| contrast_ratio(rgb(fg), rgb(*s))).fold(f32::INFINITY, f32::min);
                let white = |fill: Color32| contrast_ratio(rgb(fill), [255, 255, 255]);
                assert!(ratio(t.text) >= 7.0, "{kind:?} {p:?} text {}", ratio(t.text));
                assert!(ratio(t.tab_text_active) >= 7.0, "{kind:?} {p:?} active tab");
                assert!(ratio(t.text_dim) >= 4.5 && ratio(t.tab_text) >= 4.5 && ratio(t.icon) >= 4.5, "{kind:?} {p:?} secondary text");
                assert!(ratio(t.text_faint) >= 3.0, "{kind:?} {p:?} hints");
                assert!(
                    ratio(t.timecode) >= 4.5 && ratio(t.hot_text) >= 4.5 && ratio(t.focus) >= 4.5 && ratio(t.playhead) >= 4.5,
                    "{kind:?} {p:?} accent text"
                );
                assert!(contrast_ratio(rgb(t.bin_color), rgb(t.bin_tile_bg)) >= 3.0, "{kind:?} {p:?} bin icon");
                assert!(white(t.accent) >= 4.5 && white(t.accent_hover) >= 4.5 && white(t.track_target) >= 4.5, "{kind:?} {p:?} filled accents");
            }
        }
    }

    #[test]
    fn source_names_follow_the_preference() {
        assert_eq!(ShellSource::from_pref("auto"), Some(ShellSource::Auto));
        assert_eq!(ShellSource::from_pref("dynamic"), Some(ShellSource::Dynamic));
        assert_eq!(ShellSource::from_pref("static"), Some(ShellSource::Static));
        assert_eq!(ShellSource::from_pref("off"), None);
        assert_eq!(ShellSource::from_pref("bogus"), None);
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
//
// The few colours that are specific to this app rather than to the toolkit.
// Everything else comes from `vampir::Palette`.

use gpui::Rgba;
use vampir::{Palette, color::mix, lighting::shade};

/// Hue the palette is derived from. Cool violet: the board art is the
/// colourful part of this window, so the chrome stays out of its way.
pub const HUE: f64 = 268.0;

/// Resting colour of a keycap.
pub fn key_face(palette: Palette) -> Rgba {
    palette.control_fill
}

/// Colour for a key at `t` of its travel, 0 at rest and 1 bottomed out.
///
/// Two segments rather than one, so the actuation region and bottoming out
/// are told apart at a glance instead of being two shades of the same ramp.
pub fn travel_color(palette: Palette, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let rest = key_face(palette);
    if t < 0.6 {
        mix(rest, palette.accent, t / 0.6)
    } else {
        // Past actuation the cap keeps brightening rather than changing hue,
        // which reads as depth instead of as a different state.
        mix(palette.accent, shade(palette.accent, 0.55), (t - 0.6) / 0.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vampir::color::channels;

    fn palette() -> Palette {
        Palette::from_hue(HUE, true)
    }

    #[test]
    fn travel_is_clamped_at_both_ends() {
        let p = palette();
        assert_eq!(travel_color(p, -1.0), travel_color(p, 0.0));
        assert_eq!(travel_color(p, 2.0), travel_color(p, 1.0));
    }

    #[test]
    fn rest_matches_the_keycap_face() {
        let p = palette();
        assert_eq!(travel_color(p, 0.0), key_face(p));
    }

    #[test]
    fn actuation_point_reaches_the_accent() {
        let p = palette();
        let at_actuation = channels(travel_color(p, 0.6));
        let accent = channels(p.accent);
        for i in 0..3 {
            assert!((at_actuation[i] - accent[i]).abs() < 1e-6, "channel {i}");
        }
    }

    #[test]
    fn travel_keeps_brightening_past_actuation() {
        let p = palette();
        let sum = |t: f32| channels(travel_color(p, t))[..3].iter().sum::<f32>();
        assert!(
            sum(1.0) > sum(0.6),
            "bottoming out reads brighter than actuating"
        );
    }

    #[test]
    fn both_schemes_produce_a_usable_ramp() {
        for dark in [true, false] {
            let p = Palette::from_hue(HUE, dark);
            for step in 0..=10 {
                for v in channels(travel_color(p, step as f32 / 10.0)) {
                    assert!(
                        (0.0..=1.0).contains(&v),
                        "out of gamut at {step} with dark={dark}"
                    );
                }
            }
        }
    }
}

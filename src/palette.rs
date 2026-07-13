use std::{env, f64};

use ratatui::style::Color;

pub const CANVAS: Color = Color::Rgb(7, 9, 13);
pub const PANEL: Color = Color::Rgb(15, 19, 25);
pub const PANEL_ALT: Color = Color::Rgb(20, 25, 32);
pub const BORDER: Color = Color::Rgb(48, 57, 68);
pub const TEXT: Color = Color::Rgb(229, 235, 241);
pub const MUTED: Color = Color::Rgb(133, 145, 158);
pub const CYAN: Color = Color::Rgb(80, 213, 235);
pub const AMBER: Color = Color::Rgb(255, 190, 64);

const DEFAULT_STOPS: [(u8, u8, u8); 9] = [
    (255, 59, 48),
    (214, 45, 32),
    (155, 37, 27),
    (91, 41, 38),
    (48, 52, 59),
    (33, 77, 50),
    (24, 114, 58),
    (33, 164, 71),
    (98, 232, 93),
];

const COLORBLIND_STOPS: [(u8, u8, u8); 9] = [
    (230, 97, 1),
    (201, 81, 19),
    (164, 66, 36),
    (110, 59, 49),
    (48, 52, 59),
    (36, 72, 90),
    (23, 102, 132),
    (23, 137, 181),
    (90, 200, 250),
];

const MONO_STOPS: [(u8, u8, u8); 9] = [
    (32, 35, 41),
    (39, 42, 48),
    (45, 48, 54),
    (52, 55, 61),
    (59, 62, 68),
    (67, 70, 76),
    (75, 78, 84),
    (84, 87, 93),
    (94, 97, 103),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Default,
    Colorblind,
    Monochrome,
}

impl Theme {
    #[must_use]
    pub fn detect() -> Self {
        if env::var_os("NO_COLOR").is_some() {
            Self::Monochrome
        } else {
            Self::Default
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HeatScale {
    extent: f64,
    theme: Theme,
}

impl HeatScale {
    #[must_use]
    pub fn from_values(
        values: impl Iterator<Item = Option<f64>>,
        floor: f64,
        theme: Theme,
    ) -> Self {
        let mut magnitudes: Vec<f64> = values
            .flatten()
            .filter(|value| value.is_finite())
            .map(f64::abs)
            .collect();
        magnitudes.sort_by(f64::total_cmp);
        let percentile = magnitudes
            .get(((magnitudes.len() as f64 * 0.9).ceil() as usize).saturating_sub(1))
            .copied()
            .unwrap_or(floor);
        Self {
            extent: percentile.max(floor),
            theme,
        }
    }

    #[must_use]
    pub fn normalized(self, value: Option<f64>) -> f64 {
        value
            .filter(|value| value.is_finite())
            .map_or(0.0, |value| (value / self.extent).clamp(-1.0, 1.0))
    }

    #[must_use]
    pub fn color(self, value: Option<f64>) -> Color {
        let stops = match self.theme {
            Theme::Default => DEFAULT_STOPS,
            Theme::Colorblind => COLORBLIND_STOPS,
            Theme::Monochrome => MONO_STOPS,
        };
        let position = (self.normalized(value) + 1.0) * 0.5 * (stops.len() - 1) as f64;
        let lower = position.floor() as usize;
        let upper = (lower + 1).min(stops.len() - 1);
        let (red, green, blue) = mix(stops[lower], stops[upper], position - lower as f64);
        Color::Rgb(red, green, blue)
    }

    #[must_use]
    pub fn text_color(self, value: Option<f64>) -> Color {
        let Color::Rgb(red, green, blue) = self.color(value) else {
            return TEXT;
        };
        let luminance =
            0.2126 * f64::from(red) + 0.7152 * f64::from(green) + 0.0722 * f64::from(blue);
        if luminance > 145.0 {
            Color::Rgb(13, 16, 20)
        } else {
            Color::Rgb(246, 248, 250)
        }
    }
}

#[must_use]
pub fn detail_tint(value: Option<f64>, theme: Theme) -> Color {
    let heat = HeatScale::from_values(std::iter::once(value), 0.02, theme).color(value);
    let Color::Rgb(red, green, blue) = heat else {
        return PANEL;
    };
    let (red, green, blue) = mix((14, 17, 22), (red, green, blue), 0.22);
    Color::Rgb(red, green, blue)
}

fn mix(left: (u8, u8, u8), right: (u8, u8, u8), amount: f64) -> (u8, u8, u8) {
    let amount = amount.clamp(0.0, 1.0);
    let channel = |left: u8, right: u8| {
        (f64::from(left) + (f64::from(right) - f64::from(left)) * amount).round() as u8
    };
    (
        channel(left.0, right.0),
        channel(left.1, right.1),
        channel(left.2, right.2),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_is_symmetric_and_clamped() {
        let scale =
            HeatScale::from_values([Some(-0.1), Some(0.1)].into_iter(), 0.005, Theme::Default);
        assert_eq!(scale.normalized(Some(-0.2)), -1.0);
        assert_eq!(scale.normalized(Some(0.2)), 1.0);
        assert_eq!(scale.normalized(None), 0.0);
        assert_ne!(scale.color(Some(-0.1)), scale.color(Some(0.1)));
    }
}

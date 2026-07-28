use std::{env, f64};

use ratatui::style::Color;

use crate::domain::Sector;

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

const DEFAULT_SECTOR_HUES: [(u8, u8, u8); 9] = [
    (255, 72, 166),
    (65, 148, 255),
    (203, 72, 255),
    (75, 111, 244),
    (115, 226, 54),
    (45, 214, 190),
    (255, 123, 48),
    (255, 82, 67),
    (255, 187, 52),
];

const COLORBLIND_SECTOR_HUES: [(u8, u8, u8); 9] = [
    (230, 159, 0),
    (86, 180, 233),
    (204, 121, 167),
    (240, 228, 66),
    (0, 114, 178),
    (0, 158, 115),
    (213, 94, 0),
    (148, 103, 189),
    (170, 175, 180),
];

const VOLUME_DARK_TEXT: Color = Color::Rgb(0, 0, 0);
const VOLUME_BRIGHT_TEXT: Color = Color::Rgb(255, 255, 255);

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

    #[must_use]
    pub fn focus_color(self, value: Option<f64>) -> Color {
        let background = self.color(value);
        if contrast_ratio(background, CANVAS) > contrast_ratio(background, CYAN) {
            CANVAS
        } else {
            CYAN
        }
    }
}

/// Sector-aware color scale for volume views.
///
/// Volumes are compared in log space between the 10th and 90th percentiles, so
/// a small number of exceptional prints cannot flatten the rest of the view.
#[derive(Debug, Clone, Copy)]
pub struct VolumeScale {
    lower_log: f64,
    upper_log: f64,
    theme: Theme,
}

impl VolumeScale {
    #[must_use]
    pub fn from_values(values: impl Iterator<Item = Option<f64>>, theme: Theme) -> Self {
        let mut logarithms: Vec<f64> = values
            .flatten()
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(f64::ln_1p)
            .collect();
        logarithms.sort_by(f64::total_cmp);

        let mut lower_log = percentile(&logarithms, 0.10).unwrap_or(0.0);
        let mut upper_log = percentile(&logarithms, 0.90).unwrap_or(1.0);
        if upper_log - lower_log <= f64::EPSILON {
            lower_log = 0.0;
            upper_log = upper_log.max(1.0);
        }

        Self {
            lower_log,
            upper_log,
            theme,
        }
    }

    #[must_use]
    pub fn normalized(self, volume: Option<f64>) -> f64 {
        let Some(volume) = volume.filter(|value| value.is_finite() && *value >= 0.0) else {
            return 0.0;
        };
        ((volume.ln_1p() - self.lower_log) / (self.upper_log - self.lower_log)).clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn color(self, sector: Option<Sector>, volume: Option<f64>) -> Color {
        let intensity = self.normalized(volume);
        let (dark, bright) = volume_stops(self.theme, sector);
        let (red, green, blue) = mix(dark, bright, intensity);
        Color::Rgb(red, green, blue)
    }

    #[must_use]
    pub fn text_color(self, sector: Option<Sector>, volume: Option<f64>) -> Color {
        readable_text_color(self.color(sector, volume))
    }

    #[must_use]
    pub fn focus_color(self, sector: Option<Sector>, volume: Option<f64>) -> Color {
        let background = self.color(sector, volume);
        let accent = [CYAN, AMBER]
            .into_iter()
            .max_by(|left, right| {
                contrast_ratio(background, *left).total_cmp(&contrast_ratio(background, *right))
            })
            .unwrap_or(CYAN);
        if contrast_ratio(background, accent) >= 4.5 {
            accent
        } else {
            readable_text_color(background)
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

fn percentile(sorted: &[f64], percentile: f64) -> Option<f64> {
    let last = sorted.len().checked_sub(1)?;
    let index = (last as f64 * percentile.clamp(0.0, 1.0)).round() as usize;
    sorted.get(index).copied()
}

fn volume_stops(theme: Theme, sector: Option<Sector>) -> ((u8, u8, u8), (u8, u8, u8)) {
    if theme == Theme::Monochrome {
        return ((32, 36, 43), (160, 166, 176));
    }
    let hues = match theme {
        Theme::Default => DEFAULT_SECTOR_HUES,
        Theme::Colorblind => COLORBLIND_SECTOR_HUES,
        Theme::Monochrome => unreachable!("monochrome handled above"),
    };
    let bright = sector
        .and_then(|sector| hues.get(sector_index(sector)).copied())
        .unwrap_or((174, 185, 198));
    (mix((11, 14, 18), bright, 0.22), bright)
}

fn sector_index(sector: Sector) -> usize {
    match sector {
        Sector::Consumer => 0,
        Sector::Services => 1,
        Sector::Healthcare => 2,
        Sector::Energy => 3,
        Sector::Technology => 4,
        Sector::Financial => 5,
        Sector::Industrial => 6,
        Sector::Materials => 7,
        Sector::Utilities => 8,
    }
}

fn readable_text_color(background: Color) -> Color {
    if contrast_ratio(background, VOLUME_DARK_TEXT) > contrast_ratio(background, VOLUME_BRIGHT_TEXT)
    {
        VOLUME_DARK_TEXT
    } else {
        VOLUME_BRIGHT_TEXT
    }
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

fn contrast_ratio(left: Color, right: Color) -> f64 {
    let left = relative_luminance(left);
    let right = relative_luminance(right);
    let (lighter, darker) = if left > right {
        (left, right)
    } else {
        (right, left)
    };
    (lighter + 0.05) / (darker + 0.05)
}

fn relative_luminance(color: Color) -> f64 {
    let Color::Rgb(red, green, blue) = color else {
        return 0.0;
    };
    let linear = |channel: u8| {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.040_45 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
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

    #[test]
    fn focused_heat_color_inverts_on_bright_extremes() {
        let scale =
            HeatScale::from_values([Some(-0.1), Some(0.1)].into_iter(), 0.005, Theme::Default);

        assert_eq!(scale.focus_color(Some(-0.1)), CANVAS);
        assert_eq!(scale.focus_color(Some(0.1)), CANVAS);
        assert_eq!(scale.focus_color(None), CYAN);
        assert!(
            contrast_ratio(scale.color(Some(0.1)), scale.focus_color(Some(0.1)))
                > contrast_ratio(scale.color(Some(0.1)), CYAN)
        );
    }

    #[test]
    fn volume_scale_uses_log_percentiles_to_limit_outliers() {
        let scale = VolumeScale::from_values(
            [
                10.0,
                20.0,
                30.0,
                40.0,
                50.0,
                60.0,
                70.0,
                80.0,
                90.0,
                1_000_000_000_000.0,
            ]
            .into_iter()
            .map(Some),
            Theme::Default,
        );

        assert_eq!(scale.normalized(Some(10.0)), 0.0);
        assert!(scale.normalized(Some(50.0)) > 0.0);
        assert!(scale.normalized(Some(50.0)) < 1.0);
        assert_eq!(scale.normalized(Some(90.0)), 1.0);
        assert_eq!(scale.normalized(Some(1_000_000_000_000.0)), 1.0);
        assert_eq!(scale.normalized(Some(-1.0)), 0.0);
        assert_eq!(scale.normalized(Some(f64::NAN)), 0.0);
        assert_eq!(scale.normalized(None), 0.0);
    }

    #[test]
    fn volume_sector_hues_are_distinct_and_brighten_with_volume() {
        let scale = VolumeScale::from_values(
            [Some(1_000.0), Some(1_000_000.0)].into_iter(),
            Theme::Default,
        );
        let high_colors = Sector::ALL
            .into_iter()
            .map(|sector| scale.color(Some(sector), Some(1_000_000.0)))
            .collect::<Vec<_>>();

        for (index, sector) in Sector::ALL.into_iter().enumerate() {
            let low = scale.color(Some(sector), Some(1_000.0));
            let high = high_colors[index];
            assert!(
                relative_luminance(high) > relative_luminance(low),
                "{} volume hue did not brighten",
                sector.label()
            );
            for other in high_colors.iter().skip(index + 1) {
                assert_ne!(high, *other);
            }
        }
    }

    #[test]
    fn monochrome_volume_scale_removes_hue_but_preserves_intensity() {
        let scale = VolumeScale::from_values(
            [Some(100.0), Some(1_000_000.0)].into_iter(),
            Theme::Monochrome,
        );
        let consumer_low = scale.color(Some(Sector::Consumer), Some(100.0));
        let technology_low = scale.color(Some(Sector::Technology), Some(100.0));
        let consumer_high = scale.color(Some(Sector::Consumer), Some(1_000_000.0));

        assert_eq!(consumer_low, technology_low);
        assert!(relative_luminance(consumer_high) > relative_luminance(consumer_low));
    }

    #[test]
    fn volume_text_and_focus_colors_remain_readable() {
        for theme in [Theme::Default, Theme::Colorblind, Theme::Monochrome] {
            let scale = VolumeScale::from_values([Some(1.0), Some(1_000_000.0)].into_iter(), theme);
            for sector in Sector::ALL {
                for step in 0..=100 {
                    let logarithm = f64::from(step) / 100.0 * 1_000_001.0_f64.ln();
                    let volume = Some(logarithm.exp_m1());
                    let background = scale.color(Some(sector), volume);
                    assert!(
                        contrast_ratio(background, scale.text_color(Some(sector), volume)) >= 4.5
                    );
                    assert!(
                        contrast_ratio(background, scale.focus_color(Some(sector), volume)) >= 4.5
                    );
                }
            }
        }
    }
}

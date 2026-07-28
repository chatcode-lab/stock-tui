use std::collections::HashMap;

use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    domain::{MarketTile, Sector, SortMode},
    palette::{BORDER, CANVAS, CYAN, HeatScale, MUTED, PANEL, PANEL_ALT, TEXT, VolumeScale},
    ui::{
        layout::{SectorView, sector_cell_for_rank, sector_rank_for_cell, uniform_grid},
        state::{HitTarget, Route, SectorMetric, UiAction, UiState},
    },
};

pub fn render(frame: &mut Frame<'_>, state: &mut UiState, area: Rect) {
    let floor = if state.date_range == crate::domain::DateRange::Day {
        0.005
    } else {
        0.01
    };
    let values = if matches!(state.route, Route::Favorites) {
        state
            .favorite_tiles
            .iter()
            .map(|tile| tile.period_return)
            .collect::<Vec<_>>()
    } else {
        state
            .tiles
            .iter()
            .map(|tile| tile.period_return)
            .collect::<Vec<_>>()
    };
    let scale = HeatmapPalette::new(state, values.into_iter(), floor);
    frame
        .buffer_mut()
        .set_style(area, Style::default().bg(CANVAS));
    match state.route {
        Route::Overview => render_overview(frame, state, area, &scale),
        Route::Sector(_) => render_sector(frame, state, area, &scale, false),
        Route::Favorites => render_sector(frame, state, area, &scale, true),
        Route::Ticker(_) => {}
    }
}

enum HeatmapPalette {
    Performance(HeatScale),
    Volume {
        sectors: HashMap<Sector, VolumeScale>,
        fallback: VolumeScale,
    },
}

impl HeatmapPalette {
    fn new(
        state: &UiState,
        performance_values: impl Iterator<Item = Option<f64>>,
        floor: f64,
    ) -> Self {
        if state.sort != SortMode::Volume {
            return Self::Performance(HeatScale::from_values(
                performance_values,
                floor,
                state.theme,
            ));
        }

        let sectors = Sector::ALL
            .into_iter()
            .map(|sector| {
                let scale = VolumeScale::from_values(
                    state
                        .tiles
                        .iter()
                        .filter(|tile| tile.company.sector == Some(sector))
                        .map(|tile| tile.volume),
                    state.theme,
                );
                (sector, scale)
            })
            .collect();
        let fallback =
            VolumeScale::from_values(state.tiles.iter().map(|tile| tile.volume), state.theme);
        Self::Volume { sectors, fallback }
    }

    fn color(&self, tile: Option<&MarketTile>) -> Color {
        match self {
            Self::Performance(scale) => scale.color(tile.and_then(|tile| tile.period_return)),
            Self::Volume { sectors, fallback } => {
                let Some(tile) = tile else {
                    return PANEL_ALT;
                };
                let Some(volume) = tile
                    .volume
                    .filter(|volume| volume.is_finite() && *volume >= 0.0)
                else {
                    return PANEL_ALT;
                };
                tile.company
                    .sector
                    .and_then(|sector| {
                        sectors
                            .get(&sector)
                            .map(|scale| scale.color(Some(sector), Some(volume)))
                    })
                    .unwrap_or_else(|| fallback.color(None, Some(volume)))
            }
        }
    }

    fn text_color(&self, tile: &MarketTile) -> Color {
        match self {
            Self::Performance(scale) => scale.text_color(tile.period_return),
            Self::Volume { sectors, fallback } => {
                let Some(volume) = tile
                    .volume
                    .filter(|volume| volume.is_finite() && *volume >= 0.0)
                else {
                    return TEXT;
                };
                tile.company
                    .sector
                    .and_then(|sector| {
                        sectors
                            .get(&sector)
                            .map(|scale| scale.text_color(Some(sector), Some(volume)))
                    })
                    .unwrap_or_else(|| fallback.text_color(None, Some(volume)))
            }
        }
    }

    fn focus_color(&self, tile: &MarketTile) -> Color {
        match self {
            Self::Performance(scale) => scale.focus_color(tile.period_return),
            Self::Volume { sectors, fallback } => {
                let Some(volume) = tile
                    .volume
                    .filter(|volume| volume.is_finite() && *volume >= 0.0)
                else {
                    return CYAN;
                };
                tile.company
                    .sector
                    .and_then(|sector| {
                        sectors
                            .get(&sector)
                            .map(|scale| scale.focus_color(Some(sector), Some(volume)))
                    })
                    .unwrap_or_else(|| fallback.focus_color(None, Some(volume)))
            }
        }
    }
}

fn render_overview(frame: &mut Frame<'_>, state: &mut UiState, area: Rect, scale: &HeatmapPalette) {
    let panels = uniform_grid(area, 3, 3);
    let mut targets = Vec::new();
    let grouped: HashMap<Sector, Vec<&MarketTile>> = Sector::ALL
        .into_iter()
        .map(|sector| {
            let tiles = state
                .tiles
                .iter()
                .filter(|tile| tile.company.sector == Some(sector))
                .take(100)
                .collect();
            (sector, tiles)
        })
        .collect();

    for (sector_index, (sector, panel)) in Sector::ALL.into_iter().zip(panels).enumerate() {
        if panel.width == 0 || panel.height == 0 {
            continue;
        }
        let tiles = grouped.get(&sector).map(Vec::as_slice).unwrap_or_default();
        let selected = sector_index == state.selected_sector;
        render_sector_header(frame.buffer_mut(), panel, sector, tiles, selected);
        targets.push(HitTarget {
            rect: panel,
            action: UiAction::OpenSector(sector),
            hover_symbol: None,
        });
        if panel.height <= 1 {
            render_sector_marker(frame.buffer_mut(), panel, selected);
            continue;
        }
        let body = Rect::new(
            panel.x.saturating_add(1),
            panel.y + 1,
            panel.width.saturating_sub(1),
            panel.height - 1,
        );
        if body.height >= 10 {
            let cells = uniform_grid(body, 10, 10);
            for (rank, tile) in tiles.iter().enumerate().take(cells.len()) {
                let cell_index = sector_cell_for_rank(state.sector_view, rank, 10, 10);
                let cell = cells[cell_index];
                draw_tile(frame.buffer_mut(), cell, tile, scale, false, None);
            }
        } else {
            render_paired_rows(frame.buffer_mut(), body, tiles, scale, state.sector_view);
        }
        render_sector_marker(frame.buffer_mut(), panel, selected);
    }
    drop(grouped);
    state.hit_targets.extend(targets);
}

fn render_sector(
    frame: &mut Frame<'_>,
    state: &mut UiState,
    area: Rect,
    scale: &HeatmapPalette,
    favorites_only: bool,
) {
    let tiles: Vec<MarketTile> = state.visible_tiles().into_iter().cloned().collect();
    let columns = sector_column_count(area, tiles.len());
    state.sector_columns = columns;
    state.selected_ticker = state.selected_ticker.min(tiles.len().saturating_sub(1));
    let rows = tiles.len().div_ceil(columns).max(1);
    state.sector_rows = rows;
    let cells = uniform_grid(area, columns as u16, rows as u16);
    let sector_returns: HashMap<Sector, f64> = Sector::ALL
        .into_iter()
        .filter_map(|sector| {
            aggregate_return(
                state
                    .tiles
                    .iter()
                    .filter(|tile| tile.company.sector == Some(sector)),
            )
            .map(|value| (sector, value))
        })
        .collect();
    for (rank, tile) in tiles.iter().enumerate() {
        let cell_index = sector_cell_for_rank(state.sector_view, rank, columns, rows);
        let Some(&cell) = cells.get(cell_index) else {
            continue;
        };
        let focused = rank == state.selected_ticker;
        let sector_return = tile
            .company
            .sector
            .and_then(|sector| sector_returns.get(&sector).copied());
        let metric_width = if favorite_frame_fits(tile, cell, true) {
            cell.width.saturating_sub(2)
        } else {
            cell.width
        };
        let metric = format_sector_metric(
            tile,
            state.sector_metric,
            sector_return,
            usize::from(metric_width),
        );
        draw_tile(
            frame.buffer_mut(),
            cell,
            tile,
            scale,
            focused,
            Some(&metric),
        );
        state.register(
            cell,
            UiAction::OpenTicker(tile.company.symbol.clone()),
            Some(tile.company.symbol.clone()),
        );
    }
    if tiles.is_empty() {
        let message = if favorites_only {
            "No starred tickers yet  ·  press f on any ticker"
        } else {
            "This sector is waiting for cached market data"
        };
        put_centered(
            frame.buffer_mut(),
            area,
            message,
            Style::default().fg(MUTED).bg(CANVAS),
        );
    }
}

fn sector_column_count(area: Rect, tile_count: usize) -> usize {
    let ten_column_rows = tile_count.div_ceil(10).max(1);
    let ten_columns_fit = area.width / 10 >= 6;
    let two_lines_fit = usize::from(area.height) / ten_column_rows >= 2;
    if ten_columns_fit && two_lines_fit {
        10
    } else {
        usize::from((area.width / 7).clamp(3, 10))
    }
}

fn render_sector_header(
    buffer: &mut Buffer,
    area: Rect,
    sector: Sector,
    tiles: &[&MarketTile],
    selected: bool,
) {
    let aggregate = aggregate_return(tiles.iter().copied());
    let label = aggregate.map_or_else(
        || format!(" {} -- ", sector.label()),
        |value| format!(" {} {value:+.2}% ", sector.label(), value = value * 100.0),
    );
    let style = Style::default()
        .fg(if selected { CYAN } else { TEXT })
        .bg(PANEL)
        .add_modifier(Modifier::BOLD);
    buffer.set_style(Rect::new(area.x, area.y, area.width, 1), style);
    buffer.set_stringn(
        area.x + 1,
        area.y,
        label,
        area.width.saturating_sub(1) as usize,
        style,
    );
}

fn render_sector_marker(buffer: &mut Buffer, area: Rect, selected: bool) {
    for y in area.y..area.bottom() {
        let cell = &mut buffer[(area.x, y)];
        cell.set_symbol(if selected { "▌" } else { "│" })
            .set_fg(if selected { CYAN } else { BORDER });
        if y == area.y {
            cell.set_bg(PANEL);
        } else {
            cell.set_bg(CANVAS);
        }
    }
}

fn render_paired_rows(
    buffer: &mut Buffer,
    area: Rect,
    tiles: &[&MarketTile],
    scale: &HeatmapPalette,
    view: SectorView,
) {
    let columns = uniform_grid(Rect::new(area.x, area.y, area.width, 1), 10, 1);
    for compact_row in 0..area.height.min(5) {
        for (column, column_rect) in columns.iter().enumerate() {
            let top_cell = usize::from(compact_row) * 20 + column;
            let bottom_cell = top_cell + 10;
            let tile_for_cell = |cell| {
                sector_rank_for_cell(view, cell, tiles.len().min(100), 10, 10)
                    .and_then(|rank| tiles.get(rank))
            };
            let top = tile_for_cell(top_cell).copied();
            let bottom = tile_for_cell(bottom_cell).copied();
            for x in column_rect.x..column_rect.right() {
                buffer[(x, area.y + compact_row)]
                    .set_symbol("▀")
                    .set_fg(scale.color(top))
                    .set_bg(scale.color(bottom));
            }
        }
    }
}

fn draw_tile(
    buffer: &mut Buffer,
    area: Rect,
    tile: &MarketTile,
    scale: &HeatmapPalette,
    focused: bool,
    metric: Option<&str>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let background = scale.color(Some(tile));
    let foreground = if focused {
        scale.focus_color(tile)
    } else {
        scale.text_color(tile)
    };
    let mut style = Style::default().fg(foreground).bg(background);
    if focused || tile.starred {
        style = style.add_modifier(Modifier::BOLD);
    }
    buffer.set_style(area, style);

    let framed = favorite_frame_fits(tile, area, metric.is_some());
    let content = if framed {
        draw_favorite_border(buffer, area, foreground, background);
        Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2)
    } else {
        area
    };

    let line_count = usize::from(metric.is_some() && content.height >= 2) + 1;
    let start_y = content.y
        + content
            .height
            .saturating_sub(u16::try_from(line_count).unwrap_or(1))
            / 2;
    let prefix = if framed && focused {
        "›"
    } else if !framed && tile.starred {
        "★"
    } else if focused {
        "›"
    } else {
        ""
    };
    draw_ticker_label(
        buffer,
        Rect::new(content.x, start_y, content.width, 1),
        prefix,
        &tile.company.symbol,
        style,
        tile.stale,
        scale.focus_color(tile),
    );

    if let Some(metric) = metric.filter(|_| content.height >= 2) {
        put_centered(
            buffer,
            Rect::new(content.x, start_y + 1, content.width, 1),
            metric,
            style,
        );
    }
}

fn favorite_frame_fits(tile: &MarketTile, area: Rect, has_metric: bool) -> bool {
    let frame_height = if has_metric { 4 } else { 3 };
    tile.starred && area.width >= 7 && area.height >= frame_height
}

fn draw_favorite_border(buffer: &mut Buffer, area: Rect, foreground: Color, background: Color) {
    let style = Style::default()
        .fg(foreground)
        .bg(background)
        .add_modifier(Modifier::BOLD);
    buffer[(area.x, area.y)].set_symbol("┌").set_style(style);
    buffer[(area.right() - 1, area.y)]
        .set_symbol("┐")
        .set_style(style);
    buffer[(area.x, area.bottom() - 1)]
        .set_symbol("└")
        .set_style(style);
    buffer[(area.right() - 1, area.bottom() - 1)]
        .set_symbol("┘")
        .set_style(style);
    for x in area.x + 1..area.right() - 1 {
        buffer[(x, area.y)].set_symbol("─").set_style(style);
        buffer[(x, area.bottom() - 1)]
            .set_symbol("─")
            .set_style(style);
    }
    for y in area.y + 1..area.bottom() - 1 {
        buffer[(area.x, y)].set_symbol("│").set_style(style);
        buffer[(area.right() - 1, y)]
            .set_symbol("│")
            .set_style(style);
    }
}

fn draw_ticker_label(
    buffer: &mut Buffer,
    area: Rect,
    prefix: &str,
    symbol: &str,
    style: Style,
    stale: bool,
    favorite_accent: Color,
) {
    let width = usize::from(area.width);
    if width == 0 {
        return;
    }

    let prefix_width = UnicodeWidthStr::width(prefix);
    let prefix = if prefix_width < width { prefix } else { "" };
    let prefix_width = UnicodeWidthStr::width(prefix);
    let symbol = truncate_to_width(symbol, width.saturating_sub(prefix_width));
    let symbol_width = UnicodeWidthStr::width(symbol.as_str());
    let used = symbol_width + prefix_width;
    let mut x = area.x + u16::try_from((width - used) / 2).unwrap_or(0);
    if !prefix.is_empty() {
        let prefix_style = if prefix == "★" {
            style.fg(favorite_accent)
        } else {
            style
        };
        buffer.set_stringn(x, area.y, prefix, width, prefix_style);
        x += u16::try_from(UnicodeWidthStr::width(prefix)).unwrap_or(0);
    }
    let ticker_style = if stale {
        style.add_modifier(Modifier::UNDERLINED)
    } else {
        style
    };
    buffer.set_stringn(
        x,
        area.y,
        symbol,
        width.saturating_sub(prefix_width),
        ticker_style,
    );
}

fn format_sector_metric(
    tile: &MarketTile,
    metric: SectorMetric,
    sector_return: Option<f64>,
    width: usize,
) -> String {
    match metric {
        SectorMetric::Price => tile
            .price
            .filter(|value| value.is_finite())
            .map_or_else(|| "--".to_owned(), |price| format_price(price, width)),
        SectorMetric::RelativeGain => tile
            .period_return
            .filter(|value| value.is_finite())
            .map_or_else(|| "--".to_owned(), |value| format_percent(value, width)),
        SectorMetric::AbsoluteGain => tile
            .absolute_change()
            .filter(|value| value.is_finite())
            .map_or_else(
                || "--".to_owned(),
                |value| format_signed_money(value, width),
            ),
        SectorMetric::SectorRelativeGain => tile
            .period_return
            .zip(sector_return)
            .map(|(value, sector)| value - sector)
            .filter(|value| value.is_finite())
            .map_or_else(|| "--".to_owned(), |value| format_percent(value, width)),
        SectorMetric::MarketCap => tile
            .company
            .market_cap
            .filter(|value| value.is_finite() && *value > 0.0)
            .map_or_else(
                || "--".to_owned(),
                |value| format_compact_metric(value, "$", "", width),
            ),
        SectorMetric::Volume => tile
            .volume
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map_or_else(
                || "--".to_owned(),
                |value| format_compact_metric(value, "", "", width),
            ),
    }
}

fn format_price(value: f64, width: usize) -> String {
    let full = if value.abs() >= 10_000.0 {
        format!("${}", format_compact(value))
    } else {
        format!("${value:.2}")
    };
    let mut candidates = vec![full];
    candidates.extend(
        [1, 0]
            .into_iter()
            .map(|precision| format!("${value:.precision$}")),
    );
    candidates.extend(compact_candidates(value, "$", ""));
    fit_metric(candidates, width)
}

fn format_percent(value: f64, width: usize) -> String {
    let value = normalized_zero(value * 100.0, 0.005);
    let mut candidates = [2, 1, 0]
        .into_iter()
        .map(|precision| format!("{value:+.precision$}%"))
        .collect::<Vec<_>>();
    let sign = if value.is_sign_negative() { "-" } else { "+" };
    candidates.extend(compact_candidates(value.abs(), sign, "%"));
    fit_metric(candidates, width)
}

fn format_signed_money(value: f64, width: usize) -> String {
    let value = normalized_zero(value, 0.005);
    let sign = if value.is_sign_negative() { '-' } else { '+' };
    let full = if value.abs() >= 10_000.0 {
        format!("{sign}${}", format_compact(value.abs()))
    } else {
        format!("{sign}${:.2}", value.abs())
    };
    let mut candidates = vec![full];
    candidates.extend(
        [1, 0]
            .into_iter()
            .map(|precision| format!("{sign}${:.precision$}", value.abs())),
    );
    candidates.extend(compact_candidates(value.abs(), &format!("{sign}$"), ""));
    fit_metric(candidates, width)
}

fn normalized_zero(value: f64, threshold: f64) -> f64 {
    if value.abs() < threshold { 0.0 } else { value }
}

fn format_compact(value: f64) -> String {
    let (scaled, suffix) = compact_parts(value);
    let precision = if scaled.abs() >= 100.0 {
        0
    } else if scaled.abs() >= 10.0 {
        1
    } else {
        2
    };
    format!("{scaled:.precision$}{suffix}")
}

fn format_compact_metric(value: f64, prefix: &str, suffix: &str, width: usize) -> String {
    let mut candidates = vec![format!("{prefix}{}{suffix}", format_compact(value))];
    candidates.extend(compact_candidates(value, prefix, suffix));
    fit_metric(candidates, width)
}

fn compact_candidates(value: f64, prefix: &str, trailing_unit: &str) -> Vec<String> {
    let (scaled, magnitude) = compact_parts(value);
    [2, 1, 0]
        .into_iter()
        .map(|precision| format!("{prefix}{scaled:.precision$}{magnitude}{trailing_unit}"))
        .collect()
}

fn compact_parts(value: f64) -> (f64, &'static str) {
    if value.abs() >= 1_000_000_000_000.0 {
        (value / 1_000_000_000_000.0, "T")
    } else if value.abs() >= 1_000_000_000.0 {
        (value / 1_000_000_000.0, "B")
    } else if value.abs() >= 1_000_000.0 {
        (value / 1_000_000.0, "M")
    } else if value.abs() >= 1_000.0 {
        (value / 1_000.0, "K")
    } else {
        (value, "")
    }
}

fn fit_metric(candidates: Vec<String>, width: usize) -> String {
    candidates
        .into_iter()
        .find(|candidate| UnicodeWidthStr::width(candidate.as_str()) <= width)
        .unwrap_or_else(|| truncate_to_width("--", width))
}

fn truncate_to_width(value: &str, width: usize) -> String {
    let mut output = value.to_owned();
    while UnicodeWidthStr::width(output.as_str()) > width {
        output.pop();
    }
    output
}

fn aggregate_return<'a>(tiles: impl IntoIterator<Item = &'a MarketTile>) -> Option<f64> {
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for tile in tiles {
        if let Some(value) = tile.period_return.filter(|value| value.is_finite()) {
            let weight = company_size_weight(tile.company.market_cap, tile.company.size_proxy);
            numerator += value * weight;
            denominator += weight;
        }
    }
    (denominator > 0.0).then_some(numerator / denominator)
}

fn company_size_weight(market_cap: Option<f64>, size_proxy: Option<f64>) -> f64 {
    market_cap
        .filter(|weight| weight.is_finite() && *weight > 0.0)
        .or_else(|| size_proxy.filter(|weight| weight.is_finite() && *weight > 0.0))
        .unwrap_or(1.0)
}

fn put_centered(buffer: &mut Buffer, area: Rect, value: &str, style: Style) {
    if area.height == 0 {
        return;
    }
    let line = centered_truncated(value, area.width as usize);
    let y = area.y + area.height / 2;
    buffer.set_stringn(area.x, y, line, area.width as usize, style);
}

fn centered_truncated(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut output = value.to_owned();
    while UnicodeWidthStr::width(output.as_str()) > width {
        output.pop();
    }
    let used = UnicodeWidthStr::width(output.as_str());
    let left = (width - used) / 2;
    let right = width - used - left;
    format!("{}{}{}", " ".repeat(left), output, " ".repeat(right))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::{benchmarks::MarketBenchmark, palette::Theme};

    fn market_tile() -> MarketTile {
        let mut company = MarketBenchmark::ALL[0].company(Utc::now());
        company.symbol = "TAP.A".to_owned();
        company.market_cap = Some(1_250_000_000_000.0);
        MarketTile {
            company,
            price: Some(125.0),
            period_start_price: Some(100.0),
            period_return: Some(0.25),
            volume: Some(987_000_000.0),
            starred: false,
            stale: false,
            updated_at: Some(Utc::now()),
        }
    }

    #[test]
    fn truncation_never_exceeds_cell() {
        for width in 1..8 {
            assert_eq!(
                UnicodeWidthStr::width(centered_truncated("★BRK.B", width).as_str()),
                width
            );
        }
    }

    #[test]
    fn aggregate_weight_prefers_market_cap_then_screened_size_proxy() {
        assert_eq!(company_size_weight(Some(20.0), Some(50.0)), 20.0);
        assert_eq!(company_size_weight(None, Some(50.0)), 50.0);
        assert_eq!(company_size_weight(None, Some(f64::NAN)), 1.0);
        assert_eq!(company_size_weight(Some(-1.0), Some(50.0)), 50.0);
    }

    #[test]
    fn eighty_column_sector_keeps_a_two_line_ten_by_ten_grid() {
        assert_eq!(sector_column_count(Rect::new(0, 0, 68, 21), 100), 10);
        assert_eq!(sector_column_count(Rect::new(0, 0, 48, 17), 100), 6);
    }

    #[test]
    fn exact_fit_ticker_is_complete_and_only_symbol_is_underlined() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 1));
        let style = Style::default().fg(TEXT).bg(CANVAS);

        draw_ticker_label(
            &mut buffer,
            Rect::new(0, 0, 6, 1),
            "★",
            "TAP.A",
            style,
            true,
            CYAN,
        );

        assert_eq!(
            (0..6).map(|x| buffer[(x, 0)].symbol()).collect::<String>(),
            "★TAP.A"
        );
        assert!(!buffer[(0, 0)].modifier.contains(Modifier::UNDERLINED));
        assert!((1..6).all(|x| buffer[(x, 0)].modifier.contains(Modifier::UNDERLINED)));
    }

    #[test]
    fn compact_favorite_keeps_its_star_before_truncating_the_ticker() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 1));

        draw_ticker_label(
            &mut buffer,
            Rect::new(0, 0, 3, 1),
            "★",
            "TAP.A",
            Style::default().fg(TEXT).bg(CANVAS),
            false,
            CYAN,
        );

        assert_eq!(
            (0..3).map(|x| buffer[(x, 0)].symbol()).collect::<String>(),
            "★TA"
        );
    }

    #[test]
    fn sector_metric_formats_each_supported_value() {
        let tile = market_tile();

        assert_eq!(
            format_sector_metric(&tile, SectorMetric::Price, None, usize::MAX),
            "$125.00"
        );
        assert_eq!(
            format_sector_metric(&tile, SectorMetric::RelativeGain, None, usize::MAX),
            "+25.00%"
        );
        assert_eq!(
            format_sector_metric(&tile, SectorMetric::AbsoluteGain, None, usize::MAX),
            "+$25.00"
        );
        assert_eq!(
            format_sector_metric(
                &tile,
                SectorMetric::SectorRelativeGain,
                Some(0.10),
                usize::MAX,
            ),
            "+15.00%"
        );
        assert_eq!(
            format_sector_metric(&tile, SectorMetric::MarketCap, None, usize::MAX),
            "$1.25T"
        );
        assert_eq!(
            format_sector_metric(&tile, SectorMetric::Volume, None, usize::MAX),
            "987M"
        );
    }

    #[test]
    fn six_column_metrics_keep_signs_and_units() {
        let tile = market_tile();

        assert_eq!(
            format_sector_metric(&tile, SectorMetric::Price, None, 6),
            "$125.0"
        );
        assert_eq!(
            format_sector_metric(&tile, SectorMetric::RelativeGain, None, 6),
            "+25.0%"
        );
        assert_eq!(
            format_sector_metric(&tile, SectorMetric::AbsoluteGain, None, 6),
            "+$25.0"
        );
        assert_eq!(
            format_sector_metric(&tile, SectorMetric::SectorRelativeGain, Some(0.10), 6),
            "+15.0%"
        );
        assert_eq!(
            format_sector_metric(&tile, SectorMetric::MarketCap, None, 6),
            "$1.25T"
        );
        assert_eq!(
            format_sector_metric(&tile, SectorMetric::Volume, None, 6),
            "987M"
        );
    }

    #[test]
    fn volume_palette_normalizes_each_sector_and_keeps_missing_data_neutral() {
        let mut consumer_low = market_tile();
        consumer_low.company.sector = Some(Sector::Consumer);
        consumer_low.company.symbol = "LOW".to_owned();
        consumer_low.volume = Some(100.0);
        let mut consumer_high = consumer_low.clone();
        consumer_high.company.symbol = "HIGH".to_owned();
        consumer_high.volume = Some(1_000_000.0);
        let mut technology_high = consumer_high.clone();
        technology_high.company.sector = Some(Sector::Technology);
        technology_high.company.symbol = "TECH".to_owned();
        let mut missing = consumer_low.clone();
        missing.company.symbol = "NONE".to_owned();
        missing.volume = None;
        let state = UiState {
            sort: SortMode::Volume,
            tiles: vec![
                consumer_low.clone(),
                consumer_high.clone(),
                technology_high.clone(),
                missing.clone(),
            ],
            theme: Theme::Default,
            ..UiState::default()
        };
        let palette = HeatmapPalette::new(&state, std::iter::empty(), 0.01);
        let consumer_scale = VolumeScale::from_values(
            [consumer_low.volume, consumer_high.volume].into_iter(),
            Theme::Default,
        );

        assert_eq!(
            palette.color(Some(&consumer_low)),
            consumer_scale.color(Some(Sector::Consumer), consumer_low.volume)
        );
        assert_eq!(
            palette.color(Some(&consumer_high)),
            consumer_scale.color(Some(Sector::Consumer), consumer_high.volume)
        );
        assert_ne!(
            palette.color(Some(&consumer_low)),
            palette.color(Some(&consumer_high))
        );
        assert_ne!(
            palette.color(Some(&consumer_high)),
            palette.color(Some(&technology_high))
        );
        assert_eq!(palette.color(Some(&missing)), PANEL_ALT);
    }

    #[test]
    fn favorite_tile_gets_a_frame_and_two_centered_lines() {
        let mut tile = market_tile();
        tile.starred = true;
        let mut buffer = Buffer::empty(Rect::new(0, 0, 11, 6));
        let scale = HeatmapPalette::Performance(HeatScale::from_values(
            std::iter::once(tile.period_return),
            0.01,
            Theme::Default,
        ));

        draw_tile(
            &mut buffer,
            Rect::new(0, 0, 11, 6),
            &tile,
            &scale,
            false,
            Some("+25.00%"),
        );

        assert_eq!(buffer[(0, 0)].symbol(), "┌");
        assert_eq!(buffer[(10, 0)].symbol(), "┐");
        assert_eq!(buffer[(0, 5)].symbol(), "└");
        assert_eq!(buffer[(10, 5)].symbol(), "┘");
        let screen = (0..6)
            .map(|y| (0..11).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>();
        assert!(screen[2].contains("TAP.A"));
        assert!(screen[3].contains("+25.00%"));
    }
}

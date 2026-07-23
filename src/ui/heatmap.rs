use std::collections::HashMap;

use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    domain::{MarketTile, Sector},
    palette::{AMBER, BORDER, CANVAS, CYAN, HeatScale, MUTED, PANEL, TEXT},
    ui::{
        layout::uniform_grid,
        state::{HitTarget, Route, UiAction, UiState},
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
    let scale = HeatScale::from_values(values.into_iter(), floor, state.theme);
    frame
        .buffer_mut()
        .set_style(area, Style::default().bg(CANVAS));
    match state.route {
        Route::Overview => render_overview(frame, state, area, scale),
        Route::Sector(_) => render_sector(frame, state, area, scale, false),
        Route::Favorites => render_sector(frame, state, area, scale, true),
        Route::Ticker(_) => {}
    }
}

fn render_overview(frame: &mut Frame<'_>, state: &mut UiState, area: Rect, scale: HeatScale) {
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
            for (index, tile) in tiles.iter().enumerate().take(cells.len()) {
                let cell = cells[index];
                draw_tile(frame.buffer_mut(), cell, tile, scale, false, false);
            }
        } else {
            render_paired_rows(frame.buffer_mut(), body, tiles, scale);
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
    scale: HeatScale,
    favorites_only: bool,
) {
    let tiles: Vec<MarketTile> = state.visible_tiles().into_iter().cloned().collect();
    let columns = if area.width >= 70 {
        10
    } else {
        usize::from((area.width / 7).clamp(3, 10))
    };
    state.sector_columns = columns;
    state.selected_ticker = state.selected_ticker.min(tiles.len().saturating_sub(1));
    let rows = tiles.len().div_ceil(columns).max(1);
    let cells = uniform_grid(area, columns as u16, rows as u16);
    for (index, tile) in tiles.iter().enumerate() {
        let cell = cells[index];
        let focused = index == state.selected_ticker;
        draw_tile(frame.buffer_mut(), cell, tile, scale, focused, true);
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

fn render_sector_header(
    buffer: &mut Buffer,
    area: Rect,
    sector: Sector,
    tiles: &[&MarketTile],
    selected: bool,
) {
    let aggregate = aggregate_return(tiles);
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

fn render_paired_rows(buffer: &mut Buffer, area: Rect, tiles: &[&MarketTile], scale: HeatScale) {
    let columns = uniform_grid(Rect::new(area.x, area.y, area.width, 1), 10, 1);
    for compact_row in 0..area.height.min(5) {
        for (column, column_rect) in columns.iter().enumerate() {
            let top_index = usize::from(compact_row) * 20 + column;
            let bottom_index = top_index + 10;
            let top = tiles.get(top_index).and_then(|tile| tile.period_return);
            let bottom = tiles.get(bottom_index).and_then(|tile| tile.period_return);
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
    scale: HeatScale,
    focused: bool,
    expanded: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let background = scale.color(tile.period_return);
    let foreground = if focused {
        scale.focus_color(tile.period_return)
    } else {
        scale.text_color(tile.period_return)
    };
    let mut style = Style::default().fg(foreground).bg(background);
    if focused || tile.starred {
        style = style.add_modifier(Modifier::BOLD);
    }
    if tile.stale {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    buffer.set_style(area, style);

    let prefix = if tile.starred {
        "★"
    } else if focused {
        "›"
    } else {
        ""
    };
    let label = format!("{prefix}{}", tile.company.symbol);
    let first_line = centered_truncated(&label, area.width as usize);
    buffer.set_stringn(area.x, area.y, first_line, area.width as usize, style);
    if expanded && area.height >= 2 {
        let change = tile.period_return.map_or_else(
            || "--".to_owned(),
            |value| format!("{:+.2}%", value * 100.0),
        );
        let second_line = centered_truncated(&change, area.width as usize);
        buffer.set_stringn(area.x, area.y + 1, second_line, area.width as usize, style);
    }
    if expanded && area.height >= 3 && area.width >= 9 {
        let price = tile
            .price
            .map_or_else(|| "--".to_owned(), |value| format!("${value:.2}"));
        let third_line = centered_truncated(&price, area.width as usize);
        buffer.set_stringn(
            area.x,
            area.y + 2,
            third_line,
            area.width as usize,
            style.add_modifier(Modifier::DIM),
        );
    }
    if tile.starred {
        buffer[(area.x, area.y)].set_fg(AMBER);
    }
}

fn aggregate_return(tiles: &[&MarketTile]) -> Option<f64> {
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for tile in tiles {
        if let Some(value) = tile.period_return {
            let weight = tile.company.market_cap.unwrap_or(1.0).max(1.0);
            numerator += value * weight;
            denominator += weight;
        }
    }
    (denominator > 0.0).then_some(numerator / denominator)
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
    use super::*;

    #[test]
    fn truncation_never_exceeds_cell() {
        for width in 1..8 {
            assert_eq!(
                UnicodeWidthStr::width(centered_truncated("★BRK.B", width).as_str()),
                width
            );
        }
    }
}

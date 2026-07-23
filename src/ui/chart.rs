use chrono::{DateTime, Local, Utc};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style},
    symbols::Marker,
    text::Line as TextLine,
    widgets::{
        Block, Borders, Paragraph,
        canvas::{Canvas, Line},
    },
};

use crate::{
    domain::{Bar, DateRange},
    palette::{BORDER, CYAN, MUTED, PANEL, TEXT},
    ui::state::UiState,
};

const TRACE_SAMPLES_PER_COLUMN: usize = 2;

pub fn render_price_volume(
    frame: &mut Frame<'_>,
    state: &mut UiState,
    area: Rect,
    bars: &[Bar],
    accent: Color,
) {
    if area.height < 5 || area.width < 10 {
        return;
    }
    if bars.is_empty() {
        frame.render_widget(
            Paragraph::new("Waiting for cached history")
                .centered()
                .style(Style::default().fg(MUTED).bg(PANEL))
                .block(Block::default().borders(Borders::ALL).border_style(BORDER)),
            area,
        );
        return;
    }
    let volume_height = volume_section_height(area.height);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(volume_height)])
        .split(area);
    let chart_area = sections[0];
    let data_low = bars
        .iter()
        .map(|bar| bar.close)
        .filter(|value| value.is_finite())
        .fold(f64::INFINITY, f64::min);
    let data_high = bars
        .iter()
        .map(|bar| bar.close)
        .filter(|value| value.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    if !data_low.is_finite() || !data_high.is_finite() {
        frame.render_widget(
            Paragraph::new("Cached history contains no valid prices")
                .centered()
                .style(Style::default().fg(MUTED).bg(PANEL))
                .block(Block::default().borders(Borders::ALL).border_style(BORDER)),
            area,
        );
        return;
    }
    let padding = ((data_high - data_low) * 0.08)
        .max(data_high.abs() * 0.002)
        .max(0.01);
    let bounds = [data_low - padding, data_high + padding];
    let y_labels = price_axis_labels(bounds, chart_area.height);
    let axis_width = axis_width(chart_area.width, &y_labels);

    let inner = chart_area.inner(Margin::new(1, 1));
    if inner.width <= axis_width || inner.height < 2 {
        frame.render_widget(
            Block::default()
                .title(TextLine::styled(
                    " PRICE ",
                    Style::default().fg(TEXT).bold(),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
                .style(Style::default().bg(PANEL)),
            chart_area,
        );
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(axis_width), Constraint::Min(1)])
        .split(rows[0]);
    let y_axis_area = columns[0];
    let plot_area = columns[1];
    let x_axis_area = Rect::new(plot_area.x, rows[1].y, plot_area.width, rows[1].height);
    state.chart_rect = Some(plot_area);

    let usable_width = usize::from(plot_area.width).max(1);
    let sampled = sample_bars(bars, usable_width);
    state.chart_sample_indices = sampled.iter().map(|(index, _)| *index).collect();
    let hover_index = state
        .detail_hover
        .map(|index| index.min(sampled.len().saturating_sub(1)));
    state.detail_hover = hover_index;
    let title_bar = hover_index
        .and_then(|index| sampled.get(index))
        .map_or_else(|| bars.last().expect("bars are non-empty"), |(_, bar)| *bar);
    let first_close = sampled
        .first()
        .map_or(title_bar.close, |(_, bar)| bar.close);
    let change = if first_close == 0.0 {
        0.0
    } else {
        title_bar.close / first_close - 1.0
    };
    let title = format!(
        " PRICE  {}  ${:.2}  {:+.2}%  H {:.2}  L {:.2} ",
        title_bar
            .timestamp
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M"),
        title_bar.close,
        change * 100.0,
        data_high,
        data_low
    );
    frame.render_widget(
        Block::default()
            .title(TextLine::styled(title, Style::default().fg(TEXT).bold()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(PANEL)),
        chart_area,
    );
    let trace_sampled = trace_bars(bars, usable_width.saturating_mul(TRACE_SAMPLES_PER_COLUMN));
    let points = normalized_price_points(&trace_sampled);
    let canvas_points = points.clone();
    let crosshair = hover_index.map(|index| normalized_position(index, sampled.len()));
    let hover_marker = crosshair
        .and_then(|position| interpolated_price(&points, position).map(|price| (position, price)));
    let grid_values = price_axis_values(bounds, y_labels.len());
    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .background_color(PANEL)
        .x_bounds([0.0, 1.0])
        .y_bounds(bounds)
        .paint(move |context| {
            for value in &grid_values {
                context.draw(&Line::new(0.0, *value, 1.0, *value, Color::Rgb(55, 64, 74)));
            }
            context.layer();
            if let Some((x, _)) = hover_marker {
                context.draw(&Line::new(x, bounds[0], x, bounds[1], CYAN));
                context.layer();
            }
            for pair in canvas_points.windows(2) {
                context.draw(&Line::new(
                    pair[0].0, pair[0].1, pair[1].0, pair[1].1, accent,
                ));
            }
        });
    frame.render_widget(canvas, plot_area);
    render_area_gradient(frame.buffer_mut(), plot_area, &points, bounds, accent);
    if let Some(marker) = hover_marker {
        render_hover_marker(frame.buffer_mut(), plot_area, marker, bounds);
    }
    render_price_axis(frame, y_axis_area, bounds, &y_labels);
    render_time_axis(frame, x_axis_area, &sampled, state.date_range);

    render_volume(frame, sections[1], plot_area, bars, accent, crosshair);
}

fn price_axis_values(bounds: [f64; 2], count: usize) -> Vec<f64> {
    if count <= 1 {
        return vec![bounds[0]];
    }
    (0..count)
        .map(|index| bounds[0] + (bounds[1] - bounds[0]) * index as f64 / (count - 1) as f64)
        .collect()
}

fn price_axis_labels(bounds: [f64; 2], height: u16) -> Vec<String> {
    let count = if height >= 15 {
        5
    } else if height >= 7 {
        3
    } else {
        2
    };
    price_axis_values(bounds, count)
        .into_iter()
        .map(format_axis_price)
        .collect()
}

fn axis_width(chart_width: u16, labels: &[String]) -> u16 {
    if chart_width < 24 {
        return 0;
    }
    labels
        .iter()
        .map(String::len)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
        .min(11) as u16
}

fn render_price_axis(frame: &mut Frame<'_>, area: Rect, bounds: [f64; 2], labels: &[String]) {
    if area.width == 0 || area.height == 0 || labels.is_empty() {
        return;
    }
    let values = price_axis_values(bounds, labels.len());
    let span = (bounds[1] - bounds[0]).max(f64::EPSILON);
    for (label, value) in labels.iter().zip(values) {
        let offset =
            ((bounds[1] - value) / span * f64::from(area.height.saturating_sub(1))).round() as u16;
        let row = Rect::new(area.x, area.y + offset, area.width, 1);
        frame.render_widget(
            Paragraph::new(label.as_str())
                .alignment(Alignment::Right)
                .style(Style::default().fg(MUTED).bg(PANEL)),
            row,
        );
    }
}

fn render_time_axis(
    frame: &mut Frame<'_>,
    area: Rect,
    sampled: &[(usize, &Bar)],
    range: DateRange,
) {
    if area.width == 0 || area.height == 0 || sampled.is_empty() {
        return;
    }
    frame
        .buffer_mut()
        .set_style(area, Style::default().fg(MUTED).bg(PANEL));
    let count = if area.width >= 72 {
        5
    } else if area.width >= 32 {
        3
    } else {
        2
    };
    for slot in 0..count {
        let sample_index = slot * (sampled.len() - 1) / (count - 1);
        let label = format_axis_time(sampled[sample_index].1.timestamp, range, area.width);
        let anchor = usize::from(area.width.saturating_sub(1)) * slot / (count - 1);
        let offset = if slot == 0 {
            0
        } else if slot == count - 1 {
            usize::from(area.width).saturating_sub(label.len())
        } else {
            anchor.saturating_sub(label.len() / 2)
        };
        let x = area.x + u16::try_from(offset).unwrap_or(area.width.saturating_sub(1));
        let available = usize::from(area.right().saturating_sub(x));
        frame.buffer_mut().set_stringn(
            x,
            area.y,
            label,
            available,
            Style::default().fg(MUTED).bg(PANEL),
        );
    }
}

fn format_axis_price(value: f64) -> String {
    let absolute = value.abs();
    if absolute >= 1_000_000.0 {
        format!("${:.2}M", value / 1_000_000.0)
    } else if absolute >= 10_000.0 {
        format!("${:.1}K", value / 1_000.0)
    } else if absolute >= 1_000.0 {
        format!("${value:.0}")
    } else if absolute >= 1.0 {
        format!("${value:.2}")
    } else {
        format!("${value:.4}")
    }
}

fn format_axis_time(timestamp: DateTime<Utc>, range: DateRange, width: u16) -> String {
    let local = timestamp.with_timezone(&Local);
    match range {
        DateRange::Day => local.format("%H:%M").to_string(),
        DateRange::Week if width >= 50 => local.format("%a %H:%M").to_string(),
        DateRange::Week => local.format("%a").to_string(),
        DateRange::Month | DateRange::ThreeMonths | DateRange::SixMonths | DateRange::Year => {
            local.format("%b %d").to_string()
        }
        DateRange::FiveYears => local.format("%b %Y").to_string(),
    }
}

fn volume_section_height(chart_height: u16) -> u16 {
    if chart_height < 10 {
        3
    } else {
        (chart_height / 5).clamp(4, 7)
    }
}

fn normalized_position(index: usize, count: usize) -> f64 {
    if count <= 1 {
        0.0
    } else {
        index.min(count - 1) as f64 / (count - 1) as f64
    }
}

fn normalized_price_points(sampled: &[(usize, &Bar)]) -> Vec<(f64, f64)> {
    let mut points: Vec<_> = sampled
        .iter()
        .enumerate()
        .map(|(index, (_, bar))| (normalized_position(index, sampled.len()), bar.close))
        .collect();
    if points.len() == 1 {
        points.push((1.0, points[0].1));
    }
    points
}

fn interpolated_price(points: &[(f64, f64)], position: f64) -> Option<f64> {
    let first = *points.first()?;
    if points.len() == 1 || position <= first.0 {
        return Some(first.1);
    }
    let last = *points.last().expect("points is non-empty");
    if position >= last.0 {
        return Some(last.1);
    }
    let upper = points.partition_point(|point| point.0 < position);
    let left = points[upper.saturating_sub(1)];
    let right = points[upper];
    let span = right.0 - left.0;
    if span <= f64::EPSILON {
        Some(right.1)
    } else {
        let amount = (position - left.0) / span;
        Some(left.1 + (right.1 - left.1) * amount)
    }
}

fn render_area_gradient(
    buffer: &mut Buffer,
    area: Rect,
    points: &[(f64, f64)],
    bounds: [f64; 2],
    accent: Color,
) {
    if area.is_empty() || points.is_empty() {
        return;
    }
    let span = bounds[1] - bounds[0];
    if !span.is_finite() || span <= 0.0 {
        return;
    }
    for column in 0..area.width {
        let Some(price) =
            braille_column_price(points, usize::from(column), usize::from(area.width))
        else {
            continue;
        };
        for row in 0..area.height {
            let cell_top = bounds[1] - span * f64::from(row) / f64::from(area.height);
            let cell_bottom = bounds[1] - span * f64::from(row + 1) / f64::from(area.height);
            let amount = area_gradient_amount(price, cell_top, cell_bottom, bounds[0]);
            if amount > 0.0 {
                buffer[(area.x + column, area.y + row)].set_bg(blend_color(PANEL, accent, amount));
            }
        }
    }
}

fn braille_column_price(points: &[(f64, f64)], column: usize, width: usize) -> Option<f64> {
    let dot_count = width.checked_mul(2)?;
    if dot_count <= 1 {
        return interpolated_price(points, 0.0);
    }
    let left_dot = column.saturating_mul(2).min(dot_count - 1);
    let right_dot = (left_dot + 1).min(dot_count - 1);
    let denominator = (dot_count - 1) as f64;
    let left = interpolated_price(points, left_dot as f64 / denominator)?;
    let right = interpolated_price(points, right_dot as f64 / denominator)?;
    Some((left + right) * 0.5)
}

fn area_gradient_amount(price: f64, cell_top: f64, cell_bottom: f64, floor: f64) -> f64 {
    const OUTER_EDGE_AMOUNT: f64 = 0.055;

    let cell_height = cell_top - cell_bottom;
    if !cell_height.is_finite() || cell_height <= 0.0 {
        return 0.0;
    }
    if price <= cell_bottom {
        let outside_distance = (cell_bottom - price) / cell_height;
        return OUTER_EDGE_AMOUNT * (1.0 - outside_distance).clamp(0.0, 1.0).powi(2);
    }

    let cell_center = (cell_top + cell_bottom) * 0.5;
    let fill_span = (price - floor).max(f64::EPSILON);
    let depth = ((price - cell_center) / fill_span).clamp(0.0, 1.0);
    let inside_amount = 0.05 + 0.30 * (1.0 - depth).powf(1.4);
    let coverage = ((price - cell_bottom) / cell_height).clamp(0.0, 1.0);
    OUTER_EDGE_AMOUNT + (inside_amount - OUTER_EDGE_AMOUNT) * coverage.powf(0.7)
}

fn render_hover_marker(
    buffer: &mut Buffer,
    area: Rect,
    (position, price): (f64, f64),
    bounds: [f64; 2],
) {
    if area.is_empty() || bounds[1] <= bounds[0] {
        return;
    }
    let x = braille_cell_offset(position, area.width, 2);
    let y_position = ((bounds[1] - price) / (bounds[1] - bounds[0])).clamp(0.0, 1.0);
    let y = braille_cell_offset(y_position, area.height, 4);
    buffer[(area.x + x, area.y + y)]
        .set_symbol("◆")
        .set_fg(CYAN);
}

fn braille_cell_offset(position: f64, cells: u16, dots_per_cell: usize) -> u16 {
    let dot_count = usize::from(cells).saturating_mul(dots_per_cell);
    if dot_count <= 1 {
        return 0;
    }
    let dot = (position.clamp(0.0, 1.0) * (dot_count - 1) as f64).round() as usize;
    u16::try_from(dot / dots_per_cell).unwrap_or(cells.saturating_sub(1))
}

fn render_volume(
    frame: &mut Frame<'_>,
    area: Rect,
    price_plot: Rect,
    bars: &[Bar],
    accent: Color,
    crosshair: Option<f64>,
) {
    let max_volume = bars
        .iter()
        .map(|bar| bar.volume)
        .filter(|volume| volume.is_finite() && *volume >= 0.0)
        .fold(0.0_f64, f64::max);
    let title = if max_volume > 0.0 {
        format!(" VOLUME  max {} ", format_compact_volume(max_volume))
    } else {
        " VOLUME ".to_owned()
    };
    frame.render_widget(
        Block::default()
            .title(TextLine::styled(title, Style::default().fg(MUTED)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(PANEL)),
        area,
    );
    let inner = area.inner(Margin::new(1, 1));
    if inner.is_empty() || max_volume <= 0.0 {
        return;
    }
    let left = price_plot.x.max(inner.x);
    let right = price_plot.right().min(inner.right());
    if right <= left {
        return;
    }
    let plot = Rect::new(left, inner.y, right - left, inner.height);
    let columns = volume_columns(bars, usize::from(plot.width));
    let selected_column = crosshair.map(|position| {
        (position.clamp(0.0, 1.0) * f64::from(plot.width.saturating_sub(1))).round() as usize
    });
    let full_height = usize::from(plot.height).saturating_mul(8);
    let buffer = frame.buffer_mut();
    for (column, volume) in columns.into_iter().enumerate() {
        let relative = (volume / max_volume).clamp(0.0, 1.0);
        let filled_eighths = if relative > 0.0 {
            (relative * full_height as f64)
                .round()
                .max(1.0)
                .min(full_height as f64) as usize
        } else {
            0
        };
        let color = if selected_column == Some(column) {
            CYAN
        } else {
            blend_color(PANEL, accent, 0.72)
        };
        for row in 0..usize::from(plot.height) {
            let from_bottom = usize::from(plot.height) - row - 1;
            let cell_eighths = filled_eighths.saturating_sub(from_bottom * 8).min(8);
            let cell = &mut buffer[(
                plot.x + u16::try_from(column).expect("volume column fits in plot width"),
                plot.y + u16::try_from(row).expect("volume row fits in plot height"),
            )];
            cell.set_symbol(volume_block(cell_eighths))
                .set_fg(color)
                .set_bg(PANEL);
        }
    }
}

fn volume_columns(bars: &[Bar], width: usize) -> Vec<f64> {
    if bars.is_empty() || width == 0 {
        return Vec::new();
    }
    if bars.len() <= width {
        return (0..width)
            .map(|column| {
                let index = column.saturating_mul(bars.len()) / width;
                let volume = bars[index.min(bars.len() - 1)].volume;
                if volume.is_finite() {
                    volume.max(0.0)
                } else {
                    0.0
                }
            })
            .collect();
    }
    (0..width)
        .map(|column| {
            let start = column.saturating_mul(bars.len()) / width;
            let end = (column + 1)
                .saturating_mul(bars.len())
                .div_ceil(width)
                .min(bars.len());
            bars[start..end]
                .iter()
                .map(|bar| bar.volume)
                .filter(|volume| volume.is_finite() && *volume > 0.0)
                .fold(0.0_f64, f64::max)
        })
        .collect()
}

fn volume_block(eighths: usize) -> &'static str {
    const BLOCKS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    BLOCKS[eighths.min(8)]
}

fn format_compact_volume(value: f64) -> String {
    if value >= 1_000_000_000.0 {
        format!("{:.2}B", value / 1_000_000_000.0)
    } else if value >= 1_000_000.0 {
        format!("{:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.1}K", value / 1_000.0)
    } else {
        format!("{value:.0}")
    }
}

fn blend_color(background: Color, foreground: Color, amount: f64) -> Color {
    let (Color::Rgb(bg_red, bg_green, bg_blue), Color::Rgb(fg_red, fg_green, fg_blue)) =
        (background, foreground)
    else {
        return foreground;
    };
    let mix = |background: u8, foreground: u8| {
        (f64::from(background)
            + (f64::from(foreground) - f64::from(background)) * amount.clamp(0.0, 1.0))
        .round() as u8
    };
    Color::Rgb(
        mix(bg_red, fg_red),
        mix(bg_green, fg_green),
        mix(bg_blue, fg_blue),
    )
}

fn trace_bars(bars: &[Bar], max_points: usize) -> Vec<(usize, &Bar)> {
    if bars.len() <= max_points {
        bars.iter().enumerate().collect()
    } else {
        sample_bars(bars, max_points)
    }
}

fn sample_bars(bars: &[Bar], width: usize) -> Vec<(usize, &Bar)> {
    if bars.is_empty() || width == 0 {
        return Vec::new();
    }
    if width <= 1 {
        return vec![(bars.len() - 1, &bars[bars.len() - 1])];
    }
    (0..width)
        .map(|position| {
            let index = position * (bars.len() - 1) / (width - 1);
            (index, &bars[index])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn bar(index: i64) -> Bar {
        Bar {
            symbol: "TEST".to_owned(),
            timeframe: "1Day".to_owned(),
            timestamp: Utc::now() + Duration::days(index),
            open: index as f64,
            high: index as f64,
            low: index as f64,
            close: index as f64,
            volume: 1.0,
            trade_count: None,
            vwap: None,
            source: "test".to_owned(),
        }
    }

    #[test]
    fn sampling_preserves_endpoints() {
        let bars: Vec<_> = (0..100).map(bar).collect();
        let sampled = sample_bars(&bars, 10);
        assert_eq!(sampled.first().unwrap().0, 0);
        assert_eq!(sampled.last().unwrap().0, 99);
        assert_eq!(sampled.len(), 10);
    }

    #[test]
    fn sampling_expands_sparse_history_to_the_plot_width() {
        let bars: Vec<_> = (0..4).map(bar).collect();
        let sampled = sample_bars(&bars, 20);

        assert_eq!(sampled.len(), 20);
        assert_eq!(sampled.first().unwrap().0, 0);
        assert_eq!(sampled.last().unwrap().0, 3);
    }

    #[test]
    fn trace_sampling_keeps_sparse_history_without_duplicate_steps() {
        let bars: Vec<_> = (0..4).map(bar).collect();
        let sampled = trace_bars(&bars, 20);

        assert_eq!(sampled.len(), 4);
        assert_eq!(
            sampled.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
    }

    #[test]
    fn normalized_price_points_span_the_full_plot() {
        let bars: Vec<_> = (0..4).map(bar).collect();
        let sampled = trace_bars(&bars, 20);
        let points = normalized_price_points(&sampled);

        assert_eq!(points.first().unwrap().0, 0.0);
        assert_eq!(points.last().unwrap().0, 1.0);
        assert_eq!(interpolated_price(&points, 0.5), Some(1.5));
    }

    #[test]
    fn area_gradient_softens_both_sides_of_the_trace_boundary() {
        let boundary = area_gradient_amount(9.9, 10.0, 9.0, 0.0);
        let just_outside = area_gradient_amount(9.9, 11.0, 10.0, 0.0);
        let far_outside = area_gradient_amount(9.9, 12.0, 11.0, 0.0);
        let deep_inside = area_gradient_amount(9.9, 5.0, 4.0, 0.0);
        let entering_from_outside = area_gradient_amount(8.999, 10.0, 9.0, 0.0);
        let entering_from_inside = area_gradient_amount(9.001, 10.0, 9.0, 0.0);

        assert!(boundary > deep_inside);
        assert!(just_outside > 0.0);
        assert_eq!(far_outside, 0.0);
        assert!((entering_from_outside - entering_from_inside).abs() < 0.01);
    }

    #[test]
    fn area_fill_samples_both_braille_dots_in_each_terminal_column() {
        let points = [(0.0, 0.0), (1.0, 90.0)];

        assert_eq!(braille_column_price(&points, 0, 2), Some(15.0));
        assert_eq!(braille_column_price(&points, 1, 2), Some(75.0));
    }

    #[test]
    fn hover_marker_maps_to_the_trace_cell() {
        let area = Rect::new(4, 3, 11, 6);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 12));

        render_hover_marker(&mut buffer, area, (0.5, 75.0), [50.0, 100.0]);

        let cell = &buffer[(9, 6)];
        assert_eq!(cell.symbol(), "◆");
        assert_eq!(cell.fg, CYAN);
    }

    #[test]
    fn marker_mapping_matches_braille_subcell_rasterization() {
        assert_eq!(braille_cell_offset(0.05, 11, 2), 0);
        assert_eq!(braille_cell_offset(0.10, 6, 4), 0);
        assert_eq!(braille_cell_offset(0.50, 11, 2), 5);
        assert_eq!(braille_cell_offset(1.00, 11, 2), 10);
    }

    #[test]
    fn volume_panel_grows_without_consuming_the_price_chart() {
        assert_eq!(volume_section_height(8), 3);
        assert_eq!(volume_section_height(20), 4);
        assert_eq!(volume_section_height(35), 7);
        assert_eq!(volume_section_height(80), 7);
    }

    #[test]
    fn sparse_volume_bars_fill_every_terminal_column() {
        let mut bars: Vec<_> = (0..2).map(bar).collect();
        bars[0].volume = 10.0;
        bars[1].volume = 20.0;

        assert_eq!(
            volume_columns(&bars, 6),
            vec![10.0, 10.0, 10.0, 20.0, 20.0, 20.0]
        );
    }

    #[test]
    fn dense_volume_bars_preserve_each_columns_peak() {
        let mut bars: Vec<_> = (0..8).map(bar).collect();
        for (index, bar) in bars.iter_mut().enumerate() {
            bar.volume = (index + 1) as f64;
        }

        assert_eq!(volume_columns(&bars, 2), vec![4.0, 8.0]);
    }

    #[test]
    fn volume_blocks_use_eighth_cell_precision() {
        assert_eq!(volume_block(0), " ");
        assert_eq!(volume_block(1), "▁");
        assert_eq!(volume_block(4), "▄");
        assert_eq!(volume_block(8), "█");
        assert_eq!(volume_block(20), "█");
    }

    #[test]
    fn price_axis_uses_readable_precision_and_suffixes() {
        assert_eq!(format_axis_price(499.184), "$499.18");
        assert_eq!(format_axis_price(12_450.0), "$12.4K");
        assert_eq!(format_axis_price(1_250_000.0), "$1.25M");
        assert_eq!(format_axis_price(0.123_456), "$0.1235");
    }

    #[test]
    fn time_axis_format_tracks_the_selected_range() {
        let timestamp = Utc::now();

        assert_eq!(
            format_axis_time(timestamp, DateRange::Day, 80).len(),
            "12:34".len()
        );
        assert_eq!(
            format_axis_time(timestamp, DateRange::Month, 80).len(),
            "Jul 23".len()
        );
        assert_eq!(
            format_axis_time(timestamp, DateRange::FiveYears, 80).len(),
            "Jul 2026".len()
        );
    }
}

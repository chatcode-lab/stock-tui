use chrono::{DateTime, Local, Utc};
use ratatui::{
    Frame,
    buffer::{Buffer, Cell},
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::Line as TextLine,
    widgets::{
        Block, Borders, Paragraph,
        canvas::{Canvas, Line, Points},
    },
};

use crate::{
    domain::{Bar, DateRange},
    palette::{BORDER, CANVAS, CYAN, MUTED, PANEL, TEXT},
    ui::state::UiState,
};

const TRACE_SAMPLES_PER_COLUMN: usize = 2;
const GRID_DOT: char = '·';
const CURSOR_DOT: char = '·';
const GRID_COLOR: Color = Color::Rgb(55, 64, 74);
pub(crate) const EMPTY_CHART_SAMPLE_INDEX: usize = usize::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChartTimeWindow {
    pub(crate) start: DateTime<Utc>,
    pub(crate) end: DateTime<Utc>,
}

impl ChartTimeWindow {
    fn position(self, timestamp: DateTime<Utc>) -> Option<f64> {
        if timestamp < self.start || timestamp > self.end {
            return None;
        }
        let span = self
            .end
            .signed_duration_since(self.start)
            .num_milliseconds();
        if span <= 0 {
            return None;
        }
        let offset = timestamp
            .signed_duration_since(self.start)
            .num_milliseconds();
        Some((offset as f64 / span as f64).clamp(0.0, 1.0))
    }

    fn timestamp_at(self, position: f64) -> Option<DateTime<Utc>> {
        let start = self.start.timestamp_millis();
        let span = self.end.timestamp_millis().checked_sub(start)?;
        if span <= 0 {
            return None;
        }
        let offset = (span as f64 * position.clamp(0.0, 1.0)).round() as i64;
        DateTime::from_timestamp_millis(start.checked_add(offset)?)
    }
}

pub(crate) fn render_price_volume(
    frame: &mut Frame<'_>,
    state: &mut UiState,
    area: Rect,
    bars: &[Bar],
    period_start: Option<(DateTime<Utc>, f64)>,
    period_end: Option<(DateTime<Utc>, f64)>,
    time_window: ChartTimeWindow,
    accent: Color,
) {
    if area.height < 5 || area.width < 10 {
        return;
    }
    let price_bars = reconciled_price_bars(bars, period_start, period_end)
        .into_iter()
        .filter(|bar| time_window.position(bar.timestamp).is_some())
        .collect::<Vec<_>>();
    if price_bars.is_empty() {
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
    let data_low = price_bars
        .iter()
        .map(|bar| bar.close)
        .filter(|value| value.is_finite())
        .fold(f64::INFINITY, f64::min);
    let data_high = price_bars
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
    let bounds = padded_price_bounds(data_low, data_high);
    let y_labels = price_axis_labels(bounds, chart_area.height);

    let inner = chart_area.inner(Margin::new(1, 1));
    if inner.width < 2 || inner.height < 2 {
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
    let plot_area = rows[0];
    let x_axis_area = Rect::new(plot_area.x, rows[1].y, plot_area.width, rows[1].height);
    state.chart_rect = Some(plot_area);

    let usable_width = usize::from(plot_area.width).max(1);
    let sampled = sample_bars_by_time(&price_bars, usable_width, time_window);
    state.chart_sample_indices = sampled
        .iter()
        .map(|sample| sample.map_or(EMPTY_CHART_SAMPLE_INDEX, |(index, _)| index))
        .collect();
    let hover_index = state
        .detail_hover
        .map(|index| index.min(sampled.len().saturating_sub(1)));
    state.detail_hover = hover_index;
    let hover_bar = hover_index
        .and_then(|index| sampled.get(index))
        .and_then(|sample| sample.map(|(_, bar)| bar));
    let title_bar =
        hover_bar.unwrap_or_else(|| price_bars.last().expect("price bars are non-empty"));
    let first_close = period_start
        .map(|(_, price)| price)
        .filter(|price| valid_price(*price))
        .or_else(|| price_bars.first().map(|bar| bar.close))
        .unwrap_or(title_bar.close);
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
    let trace_sampled = trace_bars(
        &price_bars,
        usable_width.saturating_mul(TRACE_SAMPLES_PER_COLUMN),
    );
    let typical_interval = typical_bar_interval_millis(&price_bars);
    let point_segments = timestamped_price_segments(&trace_sampled, time_window, typical_interval);
    let canvas_segments = point_segments.clone();
    let crosshair = hover_bar.and_then(|bar| time_window.position(bar.timestamp));
    let hover_marker = crosshair.and_then(|position| {
        interpolated_segment_price(&point_segments, position)
            .or_else(|| hover_bar.map(|bar| bar.close))
            .map(|price| (position, price))
    });
    let grid_values = price_axis_values(bounds, y_labels.len());
    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .background_color(PANEL)
        .x_bounds([0.0, 1.0])
        .y_bounds(bounds)
        .paint(move |context| {
            for segment in &canvas_segments {
                if segment.len() == 1 {
                    context.draw(&Points::new(segment, accent));
                } else {
                    for pair in segment.windows(2) {
                        context.draw(&Line::new(
                            pair[0].0, pair[0].1, pair[1].0, pair[1].1, accent,
                        ));
                    }
                }
            }
        });
    render_reference_grid(frame.buffer_mut(), plot_area, bounds, &grid_values);
    frame.render_widget(canvas, plot_area);
    render_area_gradient(
        frame.buffer_mut(),
        plot_area,
        &point_segments,
        bounds,
        accent,
    );
    render_price_axis(frame.buffer_mut(), plot_area, bounds, &y_labels);
    render_time_axis(frame, x_axis_area, time_window, state.date_range);
    if let (Some(marker), Some(bar)) = (hover_marker, hover_bar) {
        render_hover_labels(
            frame.buffer_mut(),
            plot_area,
            x_axis_area,
            marker,
            bar,
            bounds,
            state.date_range,
        );
        render_hover_indicator(frame.buffer_mut(), plot_area, marker, bounds);
    }

    render_volume(
        frame,
        sections[1],
        plot_area,
        bars,
        accent,
        crosshair,
        time_window,
    );
}

fn padded_price_bounds(data_low: f64, data_high: f64) -> [f64; 2] {
    let padding = ((data_high - data_low) * 0.08)
        .max(data_high.abs() * 0.002)
        .max(0.01);
    [(data_low - padding).max(0.0), data_high + padding]
}

fn sample_bars_by_time(
    bars: &[Bar],
    width: usize,
    window: ChartTimeWindow,
) -> Vec<Option<(usize, &Bar)>> {
    if width == 0 {
        return Vec::new();
    }
    let mut samples = vec![None; width];
    for (index, bar) in bars.iter().enumerate() {
        let Some(position) = window.position(bar.timestamp) else {
            continue;
        };
        let column = normalized_cell_index(position, width);
        let column_position = normalized_position(column, width);
        let candidate_distance = (position - column_position).abs();
        let replace = samples[column].is_none_or(|(_, current): (usize, &Bar)| {
            window
                .position(current.timestamp)
                .is_none_or(|current_position| {
                    candidate_distance <= (current_position - column_position).abs()
                })
        });
        if replace {
            samples[column] = Some((index, bar));
        }
    }
    samples
}

fn normalized_cell_index(position: f64, cells: usize) -> usize {
    if cells <= 1 {
        0
    } else {
        (position.clamp(0.0, 1.0) * (cells - 1) as f64).round() as usize
    }
}

fn reconciled_price_bars(
    bars: &[Bar],
    period_start: Option<(DateTime<Utc>, f64)>,
    period_end: Option<(DateTime<Utc>, f64)>,
) -> Vec<Bar> {
    let mut prices = bars.to_vec();
    if let Some((timestamp, price)) = period_start.filter(|(_, price)| valid_price(*price)) {
        match prices.first().map(|first| first.timestamp) {
            Some(first_at) if first_at == timestamp => {
                set_bar_price(
                    prices.first_mut().expect("price series has a first bar"),
                    price,
                );
            }
            Some(first_at) if timestamp < first_at => {
                let boundary = price_only_bar(
                    prices.first().expect("price series has a first bar"),
                    timestamp,
                    price,
                );
                prices.insert(0, boundary);
            }
            None => prices.push(price_only_bar_without_template(timestamp, price)),
            Some(_) => {}
        }
    }
    if let Some((timestamp, price)) = period_end.filter(|(_, price)| valid_price(*price)) {
        match prices.last().map(|last| last.timestamp) {
            Some(last_at) if last_at == timestamp => {
                set_bar_price(
                    prices.last_mut().expect("price series has a last bar"),
                    price,
                );
            }
            Some(last_at) if timestamp > last_at => {
                let boundary = price_only_bar(
                    prices.last().expect("price series has a last bar"),
                    timestamp,
                    price,
                );
                prices.push(boundary);
            }
            None => prices.push(price_only_bar_without_template(timestamp, price)),
            Some(_) => {}
        }
    }
    prices
}

fn price_only_bar(template: &Bar, timestamp: DateTime<Utc>, price: f64) -> Bar {
    let mut bar = template.clone();
    bar.timestamp = timestamp;
    bar.open = price;
    bar.high = price;
    bar.low = price;
    bar.close = price;
    bar.volume = 0.0;
    bar.trade_count = None;
    bar.vwap = None;
    bar.source = "resolved-price-endpoint".to_owned();
    bar
}

fn price_only_bar_without_template(timestamp: DateTime<Utc>, price: f64) -> Bar {
    Bar {
        symbol: String::new(),
        timeframe: String::new(),
        timestamp,
        open: price,
        high: price,
        low: price,
        close: price,
        volume: 0.0,
        trade_count: None,
        vwap: None,
        source: "resolved-price-endpoint".to_owned(),
    }
}

fn set_bar_price(bar: &mut Bar, price: f64) {
    bar.close = price;
    if !bar.high.is_finite() || price > bar.high {
        bar.high = price;
    }
    if !bar.low.is_finite() || price < bar.low {
        bar.low = price;
    }
}

fn valid_price(price: f64) -> bool {
    price.is_finite() && price > 0.0
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

fn render_price_axis(buffer: &mut Buffer, area: Rect, bounds: [f64; 2], labels: &[String]) {
    if area.width < 24 || area.height == 0 || labels.is_empty() {
        return;
    }
    let values = price_axis_values(bounds, labels.len());
    let span = (bounds[1] - bounds[0]).max(f64::EPSILON);
    for (label, value) in labels.iter().zip(values) {
        let position = ((bounds[1] - value) / span).clamp(0.0, 1.0);
        let row = area.y + braille_cell_offset(position, area.height, 4);
        let label = format!("{label} ");
        buffer.set_stringn(
            area.x,
            row,
            label,
            usize::from(area.width),
            Style::default().fg(MUTED).bg(PANEL),
        );
    }
}

fn render_reference_grid(buffer: &mut Buffer, area: Rect, bounds: [f64; 2], values: &[f64]) {
    if area.is_empty() || bounds[1] <= bounds[0] {
        return;
    }
    // Full-width Braille runs can accumulate fallback-font advance errors in browser terminals.
    let span = bounds[1] - bounds[0];
    for value in values {
        let position = ((bounds[1] - value) / span).clamp(0.0, 1.0);
        let row = area.y + braille_cell_offset(position, area.height, 4);
        for column in area.left()..area.right() {
            buffer[(column, row)].set_char(GRID_DOT).set_fg(GRID_COLOR);
        }
    }
}

fn render_time_axis(frame: &mut Frame<'_>, area: Rect, window: ChartTimeWindow, range: DateRange) {
    if area.width == 0 || area.height == 0 || window.timestamp_at(0.0).is_none() {
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
        let position = slot as f64 / (count - 1) as f64;
        let Some(timestamp) = window.timestamp_at(position) else {
            continue;
        };
        let label = format_axis_time(timestamp, range, area.width);
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
        DateRange::TwoYears | DateRange::FiveYears | DateRange::TenYears | DateRange::All => {
            local.format("%b %Y").to_string()
        }
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

#[cfg(test)]
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

fn interpolated_price_within(points: &[(f64, f64)], position: f64) -> Option<f64> {
    let first = points.first()?;
    let last = points.last()?;
    if position < first.0 || position > last.0 {
        return None;
    }
    interpolated_price(points, position)
}

fn interpolated_segment_price(point_segments: &[Vec<(f64, f64)>], position: f64) -> Option<f64> {
    point_segments
        .iter()
        .find_map(|points| interpolated_price_within(points, position))
}

fn render_area_gradient(
    buffer: &mut Buffer,
    area: Rect,
    point_segments: &[Vec<(f64, f64)>],
    bounds: [f64; 2],
    accent: Color,
) {
    if area.is_empty() || point_segments.is_empty() {
        return;
    }
    let span = bounds[1] - bounds[0];
    if !span.is_finite() || span <= 0.0 {
        return;
    }
    for column in 0..area.width {
        let Some(price) = point_segments.iter().find_map(|points| {
            braille_column_price(points, usize::from(column), usize::from(area.width))
        }) else {
            continue;
        };
        for row in 0..area.height {
            let cell_top = bounds[1] - span * f64::from(row) / f64::from(area.height);
            let cell_bottom = bounds[1] - span * f64::from(row + 1) / f64::from(area.height);
            let amount = area_gradient_amount(price, cell_top, cell_bottom, bounds[0], bounds[1]);
            if amount > 0.0 {
                buffer[(area.x + column, area.y + row)].set_bg(blend_color(PANEL, accent, amount));
            }
        }
    }
}

fn braille_column_price(points: &[(f64, f64)], column: usize, width: usize) -> Option<f64> {
    if points.len() == 1 {
        return (normalized_cell_index(points[0].0, width) == column).then_some(points[0].1);
    }
    let dot_count = width.checked_mul(2)?;
    if dot_count <= 1 {
        return interpolated_price_within(points, 0.0);
    }
    let left_dot = column.saturating_mul(2).min(dot_count - 1);
    let right_dot = (left_dot + 1).min(dot_count - 1);
    let denominator = (dot_count - 1) as f64;
    let left_position = left_dot as f64 / denominator;
    let right_position = right_dot as f64 / denominator;
    let left = interpolated_price_within(points, left_position);
    let right = interpolated_price_within(points, right_position);
    match (left, right) {
        (Some(left), Some(right)) => Some((left + right) * 0.5),
        (Some(price), None) | (None, Some(price)) => Some(price),
        (None, None) => {
            let first = points.first()?;
            let last = points.last()?;
            if last.0 < left_position || first.0 > right_position {
                None
            } else {
                let sample_position = ((left_position + right_position) * 0.5)
                    .clamp(first.0.min(last.0), first.0.max(last.0));
                interpolated_price(points, sample_position)
            }
        }
    }
}

fn area_gradient_amount(
    price: f64,
    cell_top: f64,
    cell_bottom: f64,
    floor: f64,
    ceiling: f64,
) -> f64 {
    const OUTER_EDGE_AMOUNT: f64 = 0.055;

    let cell_height = cell_top - cell_bottom;
    let vertical_span = ceiling - floor;
    if !cell_height.is_finite()
        || cell_height <= 0.0
        || !vertical_span.is_finite()
        || vertical_span <= 0.0
    {
        return 0.0;
    }
    if price <= cell_bottom {
        let outside_distance = (cell_bottom - price) / cell_height;
        return OUTER_EDGE_AMOUNT * (1.0 - outside_distance).clamp(0.0, 1.0).powi(2);
    }

    let cell_center = (cell_top + cell_bottom) * 0.5;
    let vertical_position = ((cell_center - floor) / vertical_span).clamp(0.0, 1.0);
    let inside_amount = 0.05 + 0.30 * vertical_position.powf(1.4);
    let coverage = ((price - cell_bottom) / cell_height).clamp(0.0, 1.0);
    OUTER_EDGE_AMOUNT + (inside_amount - OUTER_EDGE_AMOUNT) * coverage.powf(0.7)
}

fn render_hover_indicator(
    buffer: &mut Buffer,
    area: Rect,
    (position, price): (f64, f64),
    bounds: [f64; 2],
) {
    if area.is_empty() || bounds[1] <= bounds[0] {
        return;
    }
    let x = terminal_cell_offset(position, area.width);
    for row in area.top()..area.bottom() {
        buffer[(area.x + x, row)]
            .set_char(CURSOR_DOT)
            .set_fg(CYAN)
            .set_style(Style::default().remove_modifier(Modifier::all()));
    }
    let y_position = ((bounds[1] - price) / (bounds[1] - bounds[0])).clamp(0.0, 1.0);
    let y = braille_cell_offset(y_position, area.height, 4);
    buffer[(area.x + x, area.y + y)]
        .set_char(CURSOR_DOT)
        .set_style(
            Style::default()
                .fg(CANVAS)
                .bg(CYAN)
                .remove_modifier(Modifier::all()),
        );
}

fn render_hover_labels(
    buffer: &mut Buffer,
    plot_area: Rect,
    x_axis_area: Rect,
    (position, intersection_price): (f64, f64),
    selected_bar: &Bar,
    bounds: [f64; 2],
    range: DateRange,
) {
    if plot_area.is_empty() || bounds[1] <= bounds[0] {
        return;
    }
    let cursor_x = terminal_cell_offset(position, plot_area.width);
    let label_style = Style::default()
        .fg(CANVAS)
        .bg(CYAN)
        .add_modifier(Modifier::BOLD);
    let price_label = format_hover_price(selected_bar.close);
    if let Some(x) = hover_label_x(plot_area, cursor_x, price_label.len(), position) {
        let y_position =
            ((bounds[1] - intersection_price) / (bounds[1] - bounds[0])).clamp(0.0, 1.0);
        let y = plot_area.y + braille_cell_offset(y_position, plot_area.height, 4);
        buffer.set_stringn(
            x,
            y,
            price_label,
            usize::from(plot_area.right() - x),
            label_style,
        );
    }

    if x_axis_area.is_empty() {
        return;
    }
    let time_label = format_axis_time(selected_bar.timestamp, range, x_axis_area.width);
    if let Some(x) = hover_label_x(x_axis_area, cursor_x, time_label.len(), position) {
        buffer.set_stringn(
            x,
            x_axis_area.y,
            time_label,
            usize::from(x_axis_area.right() - x),
            label_style,
        );
    }
}

fn hover_label_x(area: Rect, cursor_x: u16, label_width: usize, position: f64) -> Option<u16> {
    const LABEL_GAP: usize = 1;

    let width = usize::from(area.width);
    let cursor = usize::from(cursor_x.min(area.width.saturating_sub(1)));
    if label_width == 0 || label_width + LABEL_GAP >= width {
        return None;
    }
    let right = cursor + LABEL_GAP + 1;
    let right_x = (right + label_width <= width).then_some(right);
    let left_x = cursor
        .checked_sub(label_width + LABEL_GAP)
        .filter(|left| left + label_width <= width);
    let offset = if position <= 0.5 {
        right_x.or(left_x)
    } else {
        left_x.or(right_x)
    }?;
    Some(area.x + u16::try_from(offset).ok()?)
}

fn format_hover_price(value: f64) -> String {
    if value.abs() >= 1.0 {
        format!("${value:.2}")
    } else {
        format!("${value:.4}")
    }
}

fn terminal_cell_offset(position: f64, cells: u16) -> u16 {
    (position.clamp(0.0, 1.0) * f64::from(cells.saturating_sub(1))).round() as u16
}

fn braille_cell_offset(position: f64, cells: u16, dots_per_cell: usize) -> u16 {
    braille_subcell_offset(position, cells, dots_per_cell).0
}

fn braille_subcell_offset(position: f64, cells: u16, dots_per_cell: usize) -> (u16, usize) {
    let dot_count = usize::from(cells).saturating_mul(dots_per_cell);
    if dot_count <= 1 {
        return (0, 0);
    }
    let dot = (position.clamp(0.0, 1.0) * (dot_count - 1) as f64).round() as usize;
    (
        u16::try_from(dot / dots_per_cell).unwrap_or(cells.saturating_sub(1)),
        dot % dots_per_cell,
    )
}

fn render_volume(
    frame: &mut Frame<'_>,
    area: Rect,
    price_plot: Rect,
    bars: &[Bar],
    accent: Color,
    crosshair: Option<f64>,
    time_window: ChartTimeWindow,
) {
    let max_volume = bars
        .iter()
        .filter(|bar| time_window.position(bar.timestamp).is_some())
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
    let columns = volume_columns(bars, usize::from(plot.width), time_window);
    let selected_column = crosshair.map(|position| {
        (position.clamp(0.0, 1.0) * f64::from(plot.width.saturating_sub(1))).round() as usize
    });
    let buffer = frame.buffer_mut();
    for (column, volume) in columns.into_iter().enumerate() {
        let relative = (volume / max_volume).clamp(0.0, 1.0);
        let filled_eighths = volume_height_eighths(relative, usize::from(plot.height));
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
            paint_volume_cell(cell, cell_eighths, color);
        }
    }
}

fn volume_height_eighths(relative: f64, rows: usize) -> usize {
    let full_height = rows.saturating_mul(8);
    if relative <= 0.0 || !relative.is_finite() || full_height == 0 {
        return 0;
    }
    (relative.clamp(0.0, 1.0) * full_height as f64)
        .round()
        .max(1.0) as usize
}

fn volume_columns(bars: &[Bar], width: usize, window: ChartTimeWindow) -> Vec<f64> {
    if width == 0 {
        return Vec::new();
    }
    let mut columns = vec![0.0_f64; width];
    for bar in bars {
        let Some(position) = window.position(bar.timestamp) else {
            continue;
        };
        let volume = if bar.volume.is_finite() {
            bar.volume.max(0.0)
        } else {
            0.0
        };
        let column = normalized_cell_index(position, width);
        columns[column] = columns[column].max(volume);
    }
    columns
}

fn volume_block(eighths: usize) -> &'static str {
    const BLOCKS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    BLOCKS[eighths.min(8)]
}

fn paint_volume_cell(cell: &mut Cell, eighths: usize, color: Color) {
    let eighths = eighths.min(8);
    cell.set_symbol(if eighths == 8 {
        " "
    } else {
        volume_block(eighths)
    })
    .set_fg(color)
    .set_bg(if eighths == 8 { color } else { PANEL });
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

fn typical_bar_interval_millis(bars: &[Bar]) -> Option<i64> {
    let mut intervals = bars
        .windows(2)
        .filter_map(|pair| {
            let interval = pair[1]
                .timestamp
                .signed_duration_since(pair[0].timestamp)
                .num_milliseconds();
            (interval > 0).then_some(interval)
        })
        .collect::<Vec<_>>();
    if intervals.is_empty() {
        return None;
    }
    intervals.sort_unstable();
    Some(intervals[intervals.len() / 2])
}

fn timestamped_price_segments(
    sampled: &[(usize, &Bar)],
    window: ChartTimeWindow,
    typical_interval_millis: Option<i64>,
) -> Vec<Vec<(f64, f64)>> {
    const GAP_INTERVAL_MULTIPLIER: i64 = 3;

    let mut segments = Vec::new();
    let mut segment = Vec::new();
    let mut previous: Option<(usize, &Bar)> = None;
    for &(index, bar) in sampled {
        let Some(position) = window.position(bar.timestamp) else {
            continue;
        };
        let starts_new_segment = previous.is_some_and(|(previous_index, previous_bar)| {
            let interval = bar
                .timestamp
                .signed_duration_since(previous_bar.timestamp)
                .num_milliseconds();
            let observation_steps = index.saturating_sub(previous_index).max(1) as i64;
            typical_interval_millis.is_some_and(|typical| {
                let expected = typical
                    .saturating_mul(observation_steps)
                    .saturating_mul(GAP_INTERVAL_MULTIPLIER);
                interval > expected
            })
        });
        if starts_new_segment && !segment.is_empty() {
            segments.push(segment);
            segment = Vec::new();
        }
        segment.push((position, bar.close));
        previous = Some((index, bar));
    }
    if !segment.is_empty() {
        segments.push(segment);
    }
    segments
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
    use ratatui::{symbols::braille::BRAILLE, widgets::Widget};

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

    fn covering_window(bars: &[Bar]) -> ChartTimeWindow {
        ChartTimeWindow {
            start: bars.first().expect("test bars are non-empty").timestamp,
            end: bars.last().expect("test bars are non-empty").timestamp,
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
    fn timestamp_mapping_preserves_observation_gaps_and_blank_tail() {
        let bars = vec![bar(0), bar(1), bar(10), bar(11)];
        let window = ChartTimeWindow {
            start: bars[0].timestamp,
            end: bars[0].timestamp + Duration::days(12),
        };
        let samples = sample_bars_by_time(&bars, 13, window);
        let trace = trace_bars(&bars, 26);
        let segments =
            timestamped_price_segments(&trace, window, typical_bar_interval_millis(&bars));
        let volumes = volume_columns(&bars, 13, window);

        assert!(samples[0].is_some());
        assert!(samples[1].is_some());
        assert!(samples[2..10].iter().all(Option::is_none));
        assert!(samples[10].is_some());
        assert!(samples[11].is_some());
        assert!(samples[12].is_none());
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].len(), 2);
        assert_eq!(segments[1].len(), 2);
        assert_eq!(&volumes[2..10], &[0.0; 8]);
        assert_eq!(volumes[12], 0.0);
        assert!(
            segments
                .iter()
                .all(|segment| braille_column_price(segment, 12, 13).is_none())
        );
    }

    #[test]
    fn chart_time_window_maps_endpoints_without_ordinal_stretching() {
        let start = DateTime::from_timestamp_millis(Utc::now().timestamp_millis())
            .expect("current timestamp is representable");
        let window = ChartTimeWindow {
            start,
            end: start + Duration::days(4),
        };

        assert_eq!(window.position(start), Some(0.0));
        assert_eq!(window.position(start + Duration::days(1)), Some(0.25));
        assert_eq!(window.position(window.end), Some(1.0));
        assert_eq!(window.timestamp_at(0.75), Some(start + Duration::days(3)));
        assert!(window.position(start - Duration::seconds(1)).is_none());
        assert!(window.position(window.end + Duration::seconds(1)).is_none());
    }

    #[test]
    fn price_series_reconciles_period_endpoints_without_changing_volume_bars() {
        let mut bars: Vec<_> = (0..2).map(bar).collect();
        bars[0].volume = 10.0;
        bars[1].volume = 20.0;
        let start_at = bars[0].timestamp - Duration::days(1);
        let end_at = bars[1].timestamp + Duration::days(1);

        let prices = reconciled_price_bars(&bars, Some((start_at, 50.0)), Some((end_at, 75.0)));

        assert_eq!(
            prices
                .iter()
                .map(|bar| (bar.timestamp, bar.close))
                .collect::<Vec<_>>(),
            [
                (start_at, 50.0),
                (bars[0].timestamp, bars[0].close),
                (bars[1].timestamp, bars[1].close),
                (end_at, 75.0),
            ]
        );
        assert_eq!(prices.first().unwrap().volume, 0.0);
        assert_eq!(prices.last().unwrap().volume, 0.0);
        assert_eq!(
            volume_columns(&bars, 2, covering_window(&bars)),
            vec![10.0, 20.0]
        );
        assert_eq!(
            bars.iter().map(|bar| bar.volume).collect::<Vec<_>>(),
            [10.0, 20.0]
        );
    }

    #[test]
    fn price_series_replaces_a_same_timestamp_bar_endpoint() {
        let bars: Vec<_> = (0..2).map(bar).collect();
        let endpoint_at = bars[1].timestamp;

        let prices = reconciled_price_bars(&bars, None, Some((endpoint_at, 125.0)));

        assert_eq!(prices.len(), bars.len());
        assert_eq!(prices.last().unwrap().close, 125.0);
        assert_eq!(bars.last().unwrap().close, 1.0);
        assert_eq!(prices.last().unwrap().volume, bars.last().unwrap().volume);
    }

    #[test]
    fn area_gradient_softens_both_sides_of_the_trace_boundary() {
        let boundary = area_gradient_amount(9.9, 10.0, 9.0, 0.0, 12.0);
        let just_outside = area_gradient_amount(9.9, 11.0, 10.0, 0.0, 12.0);
        let far_outside = area_gradient_amount(9.9, 12.0, 11.0, 0.0, 12.0);
        let deep_inside = area_gradient_amount(9.9, 5.0, 4.0, 0.0, 12.0);
        let entering_from_outside = area_gradient_amount(8.999, 10.0, 9.0, 0.0, 12.0);
        let entering_from_inside = area_gradient_amount(9.001, 10.0, 9.0, 0.0, 12.0);

        assert!(boundary > deep_inside);
        assert!(just_outside > 0.0);
        assert_eq!(far_outside, 0.0);
        assert!((entering_from_outside - entering_from_inside).abs() < 0.01);
    }

    #[test]
    fn area_gradient_interior_intensity_depends_only_on_y() {
        let below_lower_trace = area_gradient_amount(7.0, 6.0, 5.0, 0.0, 10.0);
        let below_higher_trace = area_gradient_amount(9.0, 6.0, 5.0, 0.0, 10.0);
        let lower_row = area_gradient_amount(9.0, 2.0, 1.0, 0.0, 10.0);

        assert_eq!(below_lower_trace, below_higher_trace);
        assert!(below_lower_trace > lower_row);
    }

    #[test]
    fn area_fill_samples_both_braille_dots_in_each_terminal_column() {
        let points = [(0.0, 0.0), (1.0, 90.0)];

        assert_eq!(braille_column_price(&points, 0, 2), Some(15.0));
        assert_eq!(braille_column_price(&points, 1, 2), Some(75.0));
    }

    #[test]
    fn text_grid_stays_out_of_braille_trace_masks() {
        let area = Rect::new(0, 0, 8, 4);
        let mut buffer = Buffer::empty(area);
        render_reference_grid(&mut buffer, area, [0.0, 1.0], &[0.5]);
        let grid_row = braille_cell_offset(0.5, area.height, 4);
        assert!(
            (area.left()..area.right())
                .all(|column| buffer[(column, grid_row)].symbol() == GRID_DOT.to_string())
        );

        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([0.0, 1.0])
            .y_bounds([0.0, 1.0])
            .paint(|context| {
                context.draw(&Line::new(0.5, 0.0, 0.5, 1.0, CYAN));
            });
        Widget::render(canvas, area, &mut buffer);

        let symbols: Vec<_> = (area.left()..area.right())
            .map(|column| buffer[(column, grid_row)].symbol().to_owned())
            .collect();
        assert!(symbols.iter().any(|symbol| symbol == "·"));
        assert!(symbols.iter().any(|symbol| {
            symbol
                .chars()
                .next()
                .is_some_and(|character| ('\u{2801}'..='\u{28ff}').contains(&character))
        }));
        assert!(symbols.iter().all(|symbol| {
            symbol == "·"
                || symbol
                    .chars()
                    .next()
                    .is_some_and(|character| ('\u{2801}'..='\u{28ff}').contains(&character))
        }));
    }

    #[test]
    fn price_axis_labels_overlay_braille_aligned_plot_rows() {
        let area = Rect::new(3, 2, 30, 8);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 12));
        for row in area.rows() {
            for position in row.columns() {
                buffer[position]
                    .set_char(BRAILLE[0x03])
                    .set_fg(CYAN)
                    .set_bg(Color::Rgb(20, 40, 30));
            }
        }
        let bounds = [10.0, 20.0];
        let labels = price_axis_labels(bounds, area.height);

        render_price_axis(&mut buffer, area, bounds, &labels);

        for (label, value) in labels.iter().zip(price_axis_values(bounds, labels.len())) {
            let position = (bounds[1] - value) / (bounds[1] - bounds[0]);
            let row = area.y + braille_cell_offset(position, area.height, 4);
            let rendered: String = (area.x..area.x + label.len() as u16)
                .map(|x| buffer[(x, row)].symbol())
                .collect();
            assert_eq!(rendered, *label);
            assert_eq!(buffer[(area.x, row)].bg, PANEL);
            assert_eq!(
                buffer[(area.right() - 1, row)].symbol(),
                BRAILLE[0x03].to_string()
            );
        }
    }

    #[test]
    fn hover_marker_maps_to_the_trace_cell() {
        let area = Rect::new(4, 3, 11, 6);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 12));

        render_hover_indicator(&mut buffer, area, (0.5, 75.0), [50.0, 100.0]);

        let cell = &buffer[(9, 6)];
        assert_eq!(cell.symbol(), CURSOR_DOT.to_string());
        assert_eq!(cell.fg, CANVAS);
        assert_eq!(cell.bg, CYAN);
        assert!(cell.modifier.is_empty());
    }

    #[test]
    fn hover_labels_use_the_selected_value_and_follow_the_cursor_side() {
        let plot = Rect::new(4, 2, 32, 8);
        let axis = Rect::new(plot.x, plot.bottom(), plot.width, 1);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 44, 14));
        let mut selected = bar(0);
        selected.close = 123.456;
        let bounds = [100.0, 150.0];
        let price_label = format_hover_price(selected.close);
        let time_label = format_axis_time(selected.timestamp, DateRange::Day, axis.width);

        for position in [0.2, 0.8] {
            buffer.reset();
            let cursor = terminal_cell_offset(position, plot.width);
            render_hover_labels(
                &mut buffer,
                plot,
                axis,
                (position, 125.0),
                &selected,
                bounds,
                DateRange::Day,
            );
            let price_x =
                hover_label_x(plot, cursor, price_label.len(), position).expect("price label fits");
            let time_x =
                hover_label_x(axis, cursor, time_label.len(), position).expect("time label fits");
            let marker_y = plot.y + braille_cell_offset((bounds[1] - 125.0) / 50.0, plot.height, 4);

            let rendered_price: String = (price_x..price_x + price_label.len() as u16)
                .map(|x| buffer[(x, marker_y)].symbol())
                .collect();
            let rendered_time: String = (time_x..time_x + time_label.len() as u16)
                .map(|x| buffer[(x, axis.y)].symbol())
                .collect();
            assert_eq!(rendered_price, price_label);
            assert_eq!(rendered_time, time_label);
            assert_eq!(buffer[(price_x, marker_y)].fg, CANVAS);
            assert_eq!(buffer[(price_x, marker_y)].bg, CYAN);
            if position <= 0.5 {
                assert!(price_x > plot.x + cursor);
                assert!(time_x > axis.x + cursor);
            } else {
                assert!(price_x + price_label.len() as u16 <= plot.x + cursor);
                assert!(time_x + time_label.len() as u16 <= axis.x + cursor);
            }
        }
    }

    #[test]
    fn hover_labels_stay_inside_their_areas_and_yield_to_the_indicator() {
        let plot = Rect::new(5, 2, 12, 6);
        let axis = Rect::new(plot.x, plot.bottom(), plot.width, 1);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 24, 12));
        let mut selected = bar(0);
        selected.close = 0.123_456;
        let marker = (0.0, 0.125);
        let bounds = [0.0, 0.25];

        render_hover_labels(
            &mut buffer,
            plot,
            axis,
            marker,
            &selected,
            bounds,
            DateRange::Day,
        );
        render_hover_indicator(&mut buffer, plot, marker, bounds);

        let cursor_x = plot.x + terminal_cell_offset(marker.0, plot.width);
        for row in plot.top()..plot.bottom() {
            assert_eq!(buffer[(cursor_x, row)].symbol(), CURSOR_DOT.to_string());
        }
        assert!(hover_label_x(plot, 5, usize::from(plot.width), 0.5).is_none());
        for y in buffer.area.top()..buffer.area.bottom() {
            for x in buffer.area.left()..buffer.area.right() {
                if buffer[(x, y)].bg == CYAN {
                    assert!(
                        (x >= plot.left()
                            && x < plot.right()
                            && y >= plot.top()
                            && y < plot.bottom())
                            || (x >= axis.left()
                                && x < axis.right()
                                && y >= axis.top()
                                && y < axis.bottom())
                    );
                }
            }
        }
    }

    #[test]
    fn hover_indicator_replaces_every_row_with_one_centered_dot() {
        let area = Rect::new(3, 2, 8, 5);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 16, 10));
        for row in area.top()..area.bottom() {
            for column in area.left()..area.right() {
                buffer[(column, row)]
                    .set_symbol(if row % 2 == 0 { "⠤" } else { "⠂" })
                    .set_bg(PANEL);
            }
        }

        render_hover_indicator(&mut buffer, area, (0.6, 15.0), [10.0, 20.0]);

        let x = area.x + terminal_cell_offset(0.6, area.width);
        let marker_y = area.y + braille_cell_offset(0.5, area.height, 4);
        for row in area.top()..area.bottom() {
            let cell = &buffer[(x, row)];
            assert_eq!(cell.symbol(), CURSOR_DOT.to_string());
            if row == marker_y {
                assert_eq!(cell.fg, CANVAS);
                assert_eq!(cell.bg, CYAN);
            } else {
                assert_eq!(cell.fg, CYAN);
                assert_eq!(cell.bg, PANEL);
            }
            assert!(cell.modifier.is_empty());
        }
        for row in area.top()..area.bottom() {
            assert_eq!(buffer[(x - 1, row)].bg, PANEL);
            assert_eq!(buffer[(x + 1, row)].bg, PANEL);
        }
    }

    #[test]
    fn marker_mapping_matches_braille_subcell_rasterization() {
        assert_eq!(braille_cell_offset(0.05, 11, 2), 0);
        assert_eq!(braille_cell_offset(0.10, 6, 4), 0);
        assert_eq!(braille_cell_offset(0.50, 11, 2), 5);
        assert_eq!(braille_cell_offset(1.00, 11, 2), 10);
        assert_eq!(terminal_cell_offset(0.00, 8), 0);
        assert_eq!(terminal_cell_offset(0.50, 8), 4);
        assert_eq!(terminal_cell_offset(1.00, 8), 7);
        assert_eq!(braille_subcell_offset(0.00, 8, 2).1, 0);
        assert_eq!(braille_subcell_offset(1.00, 8, 2).1, 1);
    }

    #[test]
    fn volume_panel_grows_without_consuming_the_price_chart() {
        assert_eq!(volume_section_height(8), 3);
        assert_eq!(volume_section_height(20), 4);
        assert_eq!(volume_section_height(35), 7);
        assert_eq!(volume_section_height(80), 7);
    }

    #[test]
    fn sparse_volume_bars_leave_unobserved_columns_empty() {
        let mut bars: Vec<_> = (0..2).map(bar).collect();
        bars[0].volume = 10.0;
        bars[1].volume = 20.0;

        assert_eq!(
            volume_columns(&bars, 6, covering_window(&bars)),
            vec![10.0, 0.0, 0.0, 0.0, 0.0, 20.0]
        );
    }

    #[test]
    fn dense_volume_bars_preserve_each_columns_peak() {
        let mut bars: Vec<_> = (0..8).map(bar).collect();
        for (index, bar) in bars.iter_mut().enumerate() {
            bar.volume = (index + 1) as f64;
        }

        assert_eq!(
            volume_columns(&bars, 2, covering_window(&bars)),
            vec![4.0, 8.0]
        );
    }

    #[test]
    fn volume_height_preserves_linear_eighth_cell_variation() {
        assert_eq!(volume_height_eighths(0.0, 5), 0);
        assert_eq!(volume_height_eighths(f64::NAN, 5), 0);
        assert_eq!(volume_height_eighths(f64::EPSILON, 5), 1);
        assert_eq!(volume_height_eighths(0.25, 5), 10);
        assert_eq!(volume_height_eighths(0.50, 5), 20);
        assert_eq!(volume_height_eighths(0.75, 5), 30);
        assert_eq!(volume_height_eighths(1.0, 5), 40);
    }

    #[test]
    fn volume_blocks_use_eighth_cell_precision() {
        assert_eq!(volume_block(0), " ");
        assert_eq!(volume_block(1), "▁");
        assert_eq!(volume_block(4), "▄");
        assert_eq!(volume_block(7), "▇");
        assert_eq!(volume_block(8), "█");
    }

    #[test]
    fn volume_cells_use_background_for_full_rows_and_uniform_caps() {
        let color = Color::Rgb(20, 180, 70);
        let mut full = Cell::default();
        let mut partial = Cell::default();
        let mut empty = Cell::default();

        paint_volume_cell(&mut full, 8, color);
        paint_volume_cell(&mut partial, 3, color);
        paint_volume_cell(&mut empty, 0, color);

        assert_eq!(full.symbol(), " ");
        assert_eq!(full.bg, color);
        assert_eq!(partial.symbol(), "▃");
        assert_eq!(partial.fg, color);
        assert_eq!(partial.bg, PANEL);
        assert_eq!(empty.symbol(), " ");
        assert_eq!(empty.bg, PANEL);
    }

    #[test]
    fn zero_and_nonfinite_volume_remain_empty() {
        let mut bars: Vec<_> = (0..2).map(bar).collect();
        bars[0].volume = 0.0;
        bars[1].volume = f64::NAN;

        assert_eq!(
            volume_columns(&bars, 4, covering_window(&bars)),
            vec![0.0; 4]
        );
    }

    #[test]
    fn price_axis_uses_readable_precision_and_suffixes() {
        assert_eq!(format_axis_price(499.184), "$499.18");
        assert_eq!(format_axis_price(12_450.0), "$12.4K");
        assert_eq!(format_axis_price(1_250_000.0), "$1.25M");
        assert_eq!(format_axis_price(0.123_456), "$0.1235");
        assert_eq!(format_hover_price(12_450.0), "$12450.00");
        assert_eq!(format_hover_price(0.123_456), "$0.1235");
    }

    #[test]
    fn padded_price_axis_never_extends_below_zero() {
        assert_eq!(padded_price_bounds(0.005, 0.01), [0.0, 0.02]);
        assert_eq!(padded_price_bounds(100.0, 110.0), [99.2, 110.8]);
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
        for range in [
            DateRange::TwoYears,
            DateRange::FiveYears,
            DateRange::TenYears,
            DateRange::All,
        ] {
            assert_eq!(
                format_axis_time(timestamp, range, 80).len(),
                "Jul 2026".len()
            );
        }
    }
}

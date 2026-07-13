use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    symbols::Marker,
    text::Line as TextLine,
    widgets::{
        Block, Borders, Paragraph, Sparkline,
        canvas::{Canvas, Line},
    },
};

use crate::{
    domain::Bar,
    palette::{BORDER, CYAN, MUTED, PANEL, TEXT},
    ui::state::UiState,
};

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
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(3)])
        .split(area);
    let chart_area = sections[0];
    state.chart_rect = Some(chart_area);
    let usable_width = usize::from(chart_area.width.saturating_sub(2)).max(1);
    let sampled = sample_bars(bars, usable_width);
    state.chart_sample_indices = sampled.iter().map(|(index, _)| *index).collect();

    let low = sampled
        .iter()
        .map(|(_, bar)| bar.close)
        .fold(f64::INFINITY, f64::min);
    let high = sampled
        .iter()
        .map(|(_, bar)| bar.close)
        .fold(f64::NEG_INFINITY, f64::max);
    let padding = ((high - low) * 0.05).max(high.abs() * 0.002).max(0.01);
    let x_max = (sampled.len().saturating_sub(1)).max(1) as f64;
    let hover = state
        .detail_hover
        .and_then(|index| sampled.get(index))
        .unwrap_or_else(|| sampled.last().expect("sampled data is non-empty"));
    let (_, hovered_bar) = hover;
    let first_close = sampled
        .first()
        .map_or(hovered_bar.close, |(_, bar)| bar.close);
    let change = if first_close == 0.0 {
        0.0
    } else {
        hovered_bar.close / first_close - 1.0
    };
    let title = format!(
        " PRICE  {}  ${:.2}  {:+.2}%  H {:.2}  L {:.2} ",
        hovered_bar.timestamp.format("%Y-%m-%d %H:%M"),
        hovered_bar.close,
        change * 100.0,
        high,
        low
    );
    let points: Vec<(f64, f64)> = sampled
        .iter()
        .enumerate()
        .map(|(index, (_, bar))| (index as f64, bar.close))
        .collect();
    let previous_close = state
        .detail
        .as_ref()
        .and_then(|detail| detail.snapshot.as_ref())
        .and_then(|snapshot| snapshot.previous_close);
    let crosshair = state
        .detail_hover
        .filter(|index| *index < sampled.len())
        .map(|index| index as f64);
    let canvas = Canvas::default()
        .block(
            Block::default()
                .title(TextLine::styled(title, Style::default().fg(TEXT).bold()))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER)),
        )
        .marker(Marker::Braille)
        .background_color(PANEL)
        .x_bounds([0.0, x_max])
        .y_bounds([low - padding, high + padding])
        .paint(move |context| {
            for pair in points.windows(2) {
                context.draw(&Line::new(
                    pair[0].0, pair[0].1, pair[1].0, pair[1].1, accent,
                ));
            }
            if let Some(previous) = previous_close {
                context.draw(&Line::new(0.0, previous, x_max, previous, MUTED));
            }
            if let Some(x) = crosshair {
                context.draw(&Line::new(x, low - padding, x, high + padding, CYAN));
            }
        });
    frame.render_widget(canvas, chart_area);

    let volumes: Vec<u64> = sampled
        .iter()
        .map(|(_, bar)| bar.volume.max(0.0) as u64)
        .collect();
    let volume = Sparkline::default()
        .data(&volumes)
        .style(Style::default().fg(Color::Rgb(94, 116, 139)).bg(PANEL))
        .block(
            Block::default()
                .title(TextLine::styled(" VOLUME ", Style::default().fg(MUTED)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER)),
        );
    frame.render_widget(volume, sections[1]);
}

fn sample_bars(bars: &[Bar], width: usize) -> Vec<(usize, &Bar)> {
    if bars.len() <= width {
        return bars.iter().enumerate().collect();
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
}

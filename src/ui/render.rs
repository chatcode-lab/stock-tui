use chrono::{DateTime, Local, Utc};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    benchmarks::MarketBenchmark,
    domain::{DateRange, MarketTile, NewsItem, SortMode, TickerDetail},
    palette::{AMBER, BORDER, CANVAS, CYAN, HeatScale, MUTED, PANEL, PANEL_ALT, TEXT, detail_tint},
    ui::{
        chart::{ChartTimeWindow, render_price_volume},
        heatmap,
        layout::{AppLayout, LayoutMode, uniform_grid},
        state::{DetailTab, Overlay, Route, SectorMetric, UiAction, UiState},
    },
};

pub fn render(frame: &mut Frame<'_>, state: &mut UiState) {
    state.begin_frame();
    let area = frame.area();
    let layout = AppLayout::calculate(area);
    if layout.mode == LayoutMode::TooSmall {
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new("stock-tui needs at least 60 × 20")
                .alignment(Alignment::Center)
                .style(Style::default().fg(TEXT).bg(CANVAS))
                .block(Block::default().borders(Borders::ALL).border_style(CYAN)),
            area,
        );
        return;
    }
    render_header(frame, state, layout.header);
    match state.route {
        Route::Ticker(_) => render_detail(frame, state, layout.content, layout.mode),
        _ => heatmap::render(frame, state, layout.content),
    }
    render_rail(frame, state, layout.rail);
    render_footer(frame, state, layout.footer, layout.content);
    if let Some(overlay) = state.overlay.clone() {
        render_overlay(frame, state, area, overlay);
    }
}

fn render_header(frame: &mut Frame<'_>, state: &UiState, area: Rect) {
    let route = match &state.route {
        Route::Overview => "MARKET WALL".to_owned(),
        Route::Sector(sector) => format!("{} / TOP 100", sector.label().to_uppercase()),
        Route::Ticker(symbol) => format!("{symbol} / DETAIL"),
        Route::Favorites => "STARRED TICKERS".to_owned(),
    };
    let mut left_spans = vec![Span::styled(
        " STOCK TUI ",
        Style::default().fg(CANVAS).bg(CYAN).bold(),
    )];
    if state.simulated_data {
        left_spans.push(Span::styled(
            " SIMULATED ",
            Style::default().fg(CANVAS).bg(AMBER).bold(),
        ));
    }
    left_spans.push(Span::styled(
        format!("  {route}"),
        Style::default().fg(TEXT).bold(),
    ));
    let left = Line::from(left_spans);
    let direction = if state.sort_descending { '↓' } else { '↑' };
    let compact_right = format!(
        "{}  ·  {} {direction} ",
        state.date_range,
        state.sort.label()
    );
    let detailed_right = matches!(state.route, Route::Sector(_) | Route::Favorites).then(|| {
        format!(
            "{}  ·  {} {direction}  ·  {}  ·  {} ",
            state.date_range,
            state.sort.label(),
            state.sector_metric.label(),
            state.sector_view.label()
        )
    });
    let right = match detailed_right {
        Some(text) if left.width() + text.width() <= usize::from(area.width) => text,
        _ if left.width() + compact_right.width() <= usize::from(area.width) => compact_right,
        _ => format!("{} {direction} ", state.date_range),
    };
    let split = area.width.saturating_sub(right.width() as u16);
    frame.render_widget(
        Paragraph::new(left).style(Style::default().bg(PANEL)),
        Rect::new(area.x, area.y, split, 1),
    );
    frame.render_widget(
        Paragraph::new(right)
            .alignment(Alignment::Right)
            .style(Style::default().fg(MUTED).bg(PANEL)),
        Rect::new(area.x + split, area.y, area.width - split, 1),
    );
    if area.height < 2 {
        return;
    }
    let inspector_symbol = match state.route {
        Route::Sector(_) | Route::Favorites => state.focused_symbol(),
        Route::Overview | Route::Ticker(_) => state.hovered_symbol.as_deref(),
    };
    let inspector = inspector_symbol
        .and_then(|symbol| state.tile(symbol))
        .map_or_else(
            || state.status.clone(),
            |tile| {
                let price = tile
                    .price
                    .map_or_else(|| "--".to_owned(), |value| format!("${value:.2}"));
                let change = tile.period_return.map_or_else(
                    || "--".to_owned(),
                    |value| format!("{:+.2}%", value * 100.0),
                );
                format!(
                    "{}  {}  {}  {}",
                    tile.company.symbol, tile.company.name, price, change
                )
            },
        );
    let progress = sync_progress_label(state);
    let progress_width = progress
        .as_deref()
        .map_or(0, |text| text.width() as u16 + 1)
        .min(area.width);
    let inspector_width = area.width.saturating_sub(progress_width);
    frame.render_widget(
        Paragraph::new(format!(" {inspector}")).style(Style::default().fg(MUTED).bg(PANEL_ALT)),
        Rect::new(area.x, area.y + 1, inspector_width, 1),
    );
    if let Some(progress) = progress {
        frame.render_widget(
            Paragraph::new(format!("{progress} "))
                .alignment(Alignment::Right)
                .style(Style::default().fg(MUTED).bg(PANEL_ALT)),
            Rect::new(area.x + inspector_width, area.y + 1, progress_width, 1),
        );
    }
}

fn render_rail(frame: &mut Frame<'_>, state: &mut UiState, area: Rect) {
    frame
        .buffer_mut()
        .set_style(area, Style::default().bg(PANEL));
    let mut y = area.y;
    if !matches!(state.route, Route::Overview) || state.overlay.is_some() {
        y = rail_button(frame, state, area, y, "Esc", "Back", UiAction::Back, false);
    }
    let sector_shortcut_pending = state.sector_shortcut_pending;
    y = rail_button(
        frame,
        state,
        area,
        y,
        "/",
        "Search",
        UiAction::OpenSearch,
        false,
    );
    y = rail_button(
        frame,
        state,
        area,
        y,
        "s",
        "Sort",
        UiAction::OpenSort,
        false,
    );
    y = rail_button(
        frame,
        state,
        area,
        y,
        "F",
        "Starred",
        UiAction::OpenFavorites,
        matches!(state.route, Route::Favorites),
    );
    y = rail_button(
        frame,
        state,
        area,
        y,
        "g",
        "Sectors",
        UiAction::BeginSectorShortcut,
        sector_shortcut_pending,
    );
    let show_sector_controls =
        matches!(state.route, Route::Sector(_) | Route::Favorites) && area.height >= 20;
    let show_overview_controls = matches!(state.route, Route::Overview) && area.height >= 15;
    if show_sector_controls {
        y = rail_button(
            frame,
            state,
            area,
            y,
            "i",
            compact_metric_label(state.sector_metric),
            UiAction::CycleSectorMetric,
            false,
        );
    }
    if show_sector_controls || show_overview_controls {
        y = rail_button(
            frame,
            state,
            area,
            y,
            "o",
            if state.sort_descending {
                "Order ↓"
            } else {
                "Order ↑"
            },
            UiAction::ToggleSortDirection,
            false,
        );
        y = rail_button(
            frame,
            state,
            area,
            y,
            "v",
            state.sector_view.label(),
            UiAction::ToggleSectorView,
            false,
        );
    }
    if matches!(state.route, Route::Sector(_) | Route::Ticker(_)) {
        y = rail_button(
            frame,
            state,
            area,
            y,
            "Bksp",
            "Prev",
            UiAction::PreviousView,
            false,
        );
        y = rail_button(
            frame,
            state,
            area,
            y,
            "Space",
            "Next",
            UiAction::NextView,
            false,
        );
    }
    if let Some(symbol) = state.focused_symbol().map(str::to_owned) {
        let starred = state.tile(&symbol).is_some_and(|tile| tile.starred);
        y = rail_button(
            frame,
            state,
            area,
            y,
            "f",
            if starred { "Unstar" } else { "Star" },
            UiAction::ToggleFavorite(symbol),
            starred,
        );
    }
    if y < area.bottom() {
        frame.buffer_mut().set_stringn(
            area.x + 1,
            y,
            "RANGE",
            area.width.saturating_sub(2) as usize,
            Style::default()
                .fg(MUTED)
                .bg(PANEL)
                .add_modifier(Modifier::BOLD),
        );
        y += 1;
    }
    render_range_buttons(frame, state, area, y, area.bottom().saturating_sub(3));
    let bottom = area.bottom();
    if bottom >= area.y + 3 {
        rail_button(
            frame,
            state,
            area,
            bottom - 3,
            "r",
            "Refresh",
            UiAction::Refresh,
            false,
        );
        rail_button(
            frame,
            state,
            area,
            bottom - 2,
            "S",
            "Status",
            UiAction::OpenSync,
            false,
        );
        rail_button(
            frame,
            state,
            area,
            bottom - 1,
            "?",
            "Help",
            UiAction::OpenHelp,
            false,
        );
    }
}

fn compact_metric_label(metric: SectorMetric) -> &'static str {
    match metric {
        SectorMetric::Price => "Price",
        SectorMetric::RelativeGain => "Return",
        SectorMetric::AbsoluteGain => "Δ price",
        SectorMetric::SectorRelativeGain => "Vs sector",
        SectorMetric::MarketCap => "Mkt cap",
        SectorMetric::Volume => "Volume",
    }
}

fn render_range_buttons(
    frame: &mut Frame<'_>,
    state: &mut UiState,
    rail: Rect,
    y: u16,
    limit: u16,
) {
    let available_rows = limit.saturating_sub(y);
    let columns = if usize::from(available_rows) >= DateRange::ALL.len() {
        1_u16
    } else {
        2
    };
    let rows = u16::try_from(DateRange::ALL.len().div_ceil(usize::from(columns)))
        .unwrap_or(available_rows);
    let column_width = rail.width / columns;
    for (index, range) in DateRange::ALL.into_iter().enumerate() {
        let index = u16::try_from(index).unwrap_or(u16::MAX);
        let column = index / rows;
        let row = index % rows;
        if row >= available_rows || column >= columns {
            continue;
        }
        let x = rail.x + column * column_width;
        let width = if column + 1 == columns {
            rail.right().saturating_sub(x)
        } else {
            column_width
        };
        let rect = Rect::new(x, y + row, width, 1);
        let active = state.date_range == range;
        let adds_no_history = matches!(state.route, Route::Ticker(_))
            && state
                .detail
                .as_ref()
                .is_some_and(|detail| range_adds_no_history(detail, range));
        let style = if active {
            Style::default()
                .fg(CANVAS)
                .bg(CYAN)
                .add_modifier(Modifier::BOLD)
        } else if adds_no_history {
            Style::default().fg(MUTED).bg(PANEL)
        } else {
            Style::default().fg(TEXT).bg(PANEL)
        };
        frame.buffer_mut().set_style(rect, style);
        let label = if columns == 1 {
            format!("  {}: {}", range.shortcut(), range.label())
        } else {
            format!(" {}:{}", range.shortcut(), range.label())
        };
        frame
            .buffer_mut()
            .set_stringn(rect.x, rect.y, label, rect.width as usize, style);
        state.register(rect, UiAction::SelectRange(range), None);
    }
}

fn rail_button(
    frame: &mut Frame<'_>,
    state: &mut UiState,
    rail: Rect,
    y: u16,
    icon: &str,
    label: &str,
    action: UiAction,
    active: bool,
) -> u16 {
    if y >= rail.bottom() {
        return y;
    }
    let rect = Rect::new(rail.x, y, rail.width, 1);
    let style = if active {
        Style::default()
            .fg(CANVAS)
            .bg(CYAN)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(TEXT).bg(PANEL)
    };
    frame.buffer_mut().set_style(rect, style);
    let text = if icon.is_empty() {
        format!("  {label}")
    } else {
        format!(" {icon} {label}")
    };
    frame
        .buffer_mut()
        .set_stringn(rect.x, rect.y, text, rect.width as usize, style);
    state.register(rect, action, None);
    y + 1
}

fn render_footer(frame: &mut Frame<'_>, state: &mut UiState, area: Rect, content: Rect) {
    let status_area = Rect::new(area.x, area.y, content.width, area.height);
    let version_area = Rect::new(
        content.right(),
        area.y,
        area.right().saturating_sub(content.right()),
        area.height,
    );
    if matches!(state.route, Route::Overview) {
        frame
            .buffer_mut()
            .set_style(area, Style::default().bg(PANEL_ALT));
        render_benchmark_footer(frame, state, status_area);
        render_version(frame, version_area);
        return;
    }
    let freshness = state.snapshot_checkpoint.map_or_else(
        || {
            if state.simulated_data {
                "demo cache pending".to_owned()
            } else {
                "prices not synced".to_owned()
            }
        },
        |time| {
            let label = if state.simulated_data {
                "demo cached"
            } else {
                "prices synced"
            };
            format!("{label} {}", time.with_timezone(&Local).format("%H:%M:%S"))
        },
    );
    let right = format!("{freshness}  ");
    let left_width = status_area.width.saturating_sub(right.width() as u16);
    frame.render_widget(
        Paragraph::new(format!(" {}", sync_status_label(state)))
            .style(Style::default().fg(MUTED).bg(PANEL_ALT)),
        Rect::new(status_area.x, status_area.y, left_width, 1),
    );
    frame.render_widget(
        Paragraph::new(right)
            .alignment(Alignment::Right)
            .style(Style::default().fg(MUTED).bg(PANEL_ALT)),
        Rect::new(
            status_area.x + left_width,
            status_area.y,
            status_area.width - left_width,
            1,
        ),
    );
    render_version(frame, version_area);
}

fn render_version(frame: &mut Frame<'_>, area: Rect) {
    if area.is_empty() {
        return;
    }
    frame.render_widget(
        Paragraph::new(format!("v{} ", env!("CARGO_PKG_VERSION")))
            .alignment(Alignment::Right)
            .style(Style::default().fg(MUTED).bg(PANEL_ALT)),
        area,
    );
}

fn sync_progress_label(state: &UiState) -> Option<String> {
    (state.sync.total > 0).then(|| {
        let percent = (state.sync.fraction() * 100.0).clamp(0.0, 100.0).round();
        format!(
            "{}/{} ({percent:.0}%)",
            state.sync.completed, state.sync.total
        )
    })
}

fn sync_status_label(state: &UiState) -> String {
    sync_progress_label(state).map_or_else(
        || state.sync.message.clone(),
        |progress| format!("{} · {progress}", state.sync.message),
    )
}

fn render_benchmark_footer(frame: &mut Frame<'_>, state: &mut UiState, area: Rect) {
    frame
        .buffer_mut()
        .set_style(area, Style::default().bg(PANEL_ALT));
    let floor = if state.date_range == DateRange::Day {
        0.005
    } else {
        0.01
    };
    let scale = HeatScale::from_values(
        state.benchmarks.iter().map(|tile| tile.period_return),
        floor,
        state.theme,
    );
    let cells = uniform_grid(area, MarketBenchmark::ALL.len() as u16, 1);
    let mut targets = Vec::with_capacity(MarketBenchmark::ALL.len());
    for (index, (benchmark, cell)) in MarketBenchmark::ALL.into_iter().zip(cells).enumerate() {
        let tile = state
            .benchmarks
            .iter()
            .find(|tile| tile.company.symbol == benchmark.symbol);
        let period_return = tile.and_then(|tile| tile.period_return);
        let selected = state.selected_benchmark == Some(index);
        let mut style = Style::default()
            .fg(if selected {
                scale.focus_color(period_return)
            } else {
                scale.text_color(period_return)
            })
            .bg(scale.color(period_return));
        if selected {
            style = style.add_modifier(Modifier::BOLD);
        }
        let underline_symbol = selected || tile.is_some_and(|tile| tile.stale);
        frame.render_widget(
            Paragraph::new(benchmark_footer_line(
                benchmark,
                tile,
                cell.width,
                underline_symbol,
            ))
            .centered()
            .style(style),
            cell,
        );
        targets.push((cell, benchmark.symbol.to_owned()));
    }
    for (cell, symbol) in targets {
        state.register(cell, UiAction::OpenTicker(symbol), None);
    }
}

fn benchmark_footer_line(
    benchmark: MarketBenchmark,
    tile: Option<&MarketTile>,
    width: u16,
    underline_symbol: bool,
) -> Line<'static> {
    let text = benchmark_footer_text(benchmark, tile, width);
    let Some(symbol_start) = text.find(benchmark.symbol) else {
        return Line::from(text);
    };
    let symbol_end = symbol_start + benchmark.symbol.len();
    let symbol_style = if underline_symbol {
        Style::default().add_modifier(Modifier::UNDERLINED)
    } else {
        Style::default()
    };
    Line::from(vec![
        Span::raw(text[..symbol_start].to_owned()),
        Span::styled(benchmark.symbol.to_owned(), symbol_style),
        Span::raw(text[symbol_end..].to_owned()),
    ])
}

fn benchmark_footer_text(
    benchmark: MarketBenchmark,
    tile: Option<&MarketTile>,
    width: u16,
) -> String {
    let full_label = format!("{} · {}", benchmark.label, benchmark.symbol);
    let tight_label = format!("{}·{}", benchmark.label, benchmark.symbol);
    let full_price = tile
        .and_then(|tile| tile.price)
        .map_or_else(|| "--".to_owned(), format_price);
    let compact_price = tile.and_then(|tile| tile.price).map_or_else(
        || "--".to_owned(),
        |price| {
            if price.abs() >= 1_000.0 {
                format!("${}", format_compact(price))
            } else {
                format!("${price:.0}")
            }
        },
    );
    let full_return = tile
        .and_then(|tile| tile.period_return)
        .map_or_else(|| "--".to_owned(), format_percent);
    let compact_return = tile.and_then(|tile| tile.period_return).map_or_else(
        || "--".to_owned(),
        |value| format!("{:+.1}%", value * 100.0),
    );
    let candidates = [
        format!("{full_label}  {full_price}  {full_return}"),
        format!("{tight_label} {compact_price} {compact_return}"),
        format!("{} {compact_price} {compact_return}", benchmark.symbol),
    ];
    let width = usize::from(width);
    candidates
        .iter()
        .find(|candidate| candidate.width() <= width)
        .cloned()
        .unwrap_or_else(|| truncate_to_width(&candidates[2], width))
}

fn truncate_to_width(value: &str, width: usize) -> String {
    let mut value = value.to_owned();
    while value.width() > width {
        value.pop();
    }
    value
}

fn render_detail(frame: &mut Frame<'_>, state: &mut UiState, area: Rect, mode: LayoutMode) {
    let Some(detail) = state.detail.clone() else {
        frame.render_widget(
            Paragraph::new("Loading ticker detail")
                .centered()
                .style(Style::default().fg(MUTED)),
            area,
        );
        return;
    };
    let tint = detail_tint(detail.period_return, state.theme);
    frame
        .buffer_mut()
        .set_style(area, Style::default().bg(tint));
    if mode == LayoutMode::Full {
        render_full_detail(frame, state, area, &detail, tint);
    } else {
        render_compact_detail(frame, state, area, &detail, tint);
    }
}

fn render_full_detail(
    frame: &mut Frame<'_>,
    state: &mut UiState,
    area: Rect,
    detail: &TickerDetail,
    tint: Color,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(10)])
        .split(area);
    render_detail_header(frame, state, detail, rows[0], tint);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(rows[1]);
    let description_height = if columns[0].width < 100 { 6 } else { 5 };
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(description_height)])
        .split(columns[0]);
    let accent = performance_accent(detail.period_return);
    render_price_volume(
        frame,
        state,
        left[0],
        &detail.bars,
        detail.period_start_at.zip(detail.period_start_price),
        detail.period_end_at.zip(detail.period_end_price),
        ChartTimeWindow {
            start: detail.range_start_at,
            end: detail.range_end_at,
        },
        accent,
    );
    render_description(frame, detail, left[1], tint);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(6)])
        .split(columns[1]);
    render_statistics(frame, detail, right[0], tint);
    render_news(frame, state, &detail.news, right[1], tint);
}

fn render_compact_detail(
    frame: &mut Frame<'_>,
    state: &mut UiState,
    area: Rect,
    detail: &TickerDetail,
    tint: Color,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(1),
            Constraint::Min(8),
        ])
        .split(area);
    render_detail_header(frame, state, detail, rows[0], tint);
    render_detail_tabs(frame, state, rows[1], tint);
    match state.detail_tab {
        DetailTab::Chart => render_price_volume(
            frame,
            state,
            rows[2],
            &detail.bars,
            detail.period_start_at.zip(detail.period_start_price),
            detail.period_end_at.zip(detail.period_end_price),
            ChartTimeWindow {
                start: detail.range_start_at,
                end: detail.range_end_at,
            },
            performance_accent(detail.period_return),
        ),
        DetailTab::Statistics => render_statistics(frame, detail, rows[2], tint),
        DetailTab::News => render_news(frame, state, &detail.news, rows[2], tint),
    }
}

fn render_detail_header(
    frame: &mut Frame<'_>,
    state: &mut UiState,
    detail: &TickerDetail,
    area: Rect,
    tint: Color,
) {
    let price_value = detail.period_end_price;
    let price = price_value.map_or_else(|| "--".to_owned(), |value| format!("${value:.2}"));
    let period_return = detail.period_return.map_or_else(
        || "--".to_owned(),
        |value| format!("{:+.2}%", value * 100.0),
    );
    let period_gain = price_value
        .zip(detail.period_start_price)
        .map(|(price, baseline)| price - baseline)
        .filter(|gain| gain.is_finite())
        .map_or_else(|| "--".to_owned(), format_signed_price);
    let mut classification = MarketBenchmark::for_symbol(&detail.company.symbol).map_or_else(
        || {
            let rank = state
                .detail_rank()
                .map(|(position, total)| {
                    format!("Rank {position}/{total}  ·  {}", state.sort.label())
                })
                .or_else(|| {
                    detail
                        .sector_rank
                        .map(|position| format!("Gain rank #{position}"))
                })
                .unwrap_or_else(|| "Outside current order".to_owned());
            let sector = detail
                .company
                .sector
                .map_or("Unclassified", |sector| sector.label());
            format!("{sector}  ·  {rank}")
        },
        |benchmark| format!("{} benchmark  ·  ETF proxy", benchmark.label),
    );
    if let Some(history) = compact_history_coverage(detail) {
        classification.push_str("  ·  ");
        classification.push_str(&history);
    }
    let favorite = if detail.starred { "★" } else { "☆" };
    let favorite_offset = detail.company.symbol.width() as u16 + 2;
    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!(" {} {favorite} ", detail.company.symbol),
                Style::default()
                    .fg(CANVAS)
                    .bg(performance_accent(detail.period_return))
                    .bold(),
            ),
            Span::styled(
                format!("  {price}  {period_gain}  {period_return}"),
                Style::default().fg(TEXT).bold(),
            ),
        ]),
        Line::styled(
            format!(" {}", detail.company.name),
            Style::default().fg(TEXT),
        ),
        Line::styled(format!(" {classification}"), Style::default().fg(MUTED)),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(tint))
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(BORDER),
            ),
        area,
    );
    if favorite_offset < area.width {
        state.register(
            Rect::new(area.x + favorite_offset, area.y, 1, 1),
            UiAction::ToggleFavorite(detail.company.symbol.clone()),
            Some(detail.company.symbol.clone()),
        );
    }
}

fn render_detail_tabs(frame: &mut Frame<'_>, state: &mut UiState, area: Rect, tint: Color) {
    let tabs = [
        (DetailTab::Chart, "Chart"),
        (DetailTab::Statistics, "Statistics"),
        (DetailTab::News, "News"),
    ];
    let widths = [
        Constraint::Percentage(33),
        Constraint::Percentage(34),
        Constraint::Percentage(33),
    ];
    let cells = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(widths)
        .split(area);
    for ((tab, label), cell) in tabs.into_iter().zip(cells.iter().copied()) {
        let active = state.detail_tab == tab;
        let style = if active {
            Style::default().fg(CANVAS).bg(CYAN).bold()
        } else {
            Style::default().fg(MUTED).bg(tint)
        };
        frame.render_widget(Paragraph::new(label).centered().style(style), cell);
        state.register(cell, UiAction::SelectDetailTab(tab), None);
    }
}

fn render_statistics(frame: &mut Frame<'_>, detail: &TickerDetail, area: Rect, tint: Color) {
    let snapshot = detail.snapshot.as_ref();
    let rows = [
        (
            "OPEN",
            snapshot.and_then(|quote| quote.open).map(format_price),
        ),
        (
            "HIGH",
            snapshot.and_then(|quote| quote.high).map(format_price),
        ),
        (
            "LOW",
            snapshot.and_then(|quote| quote.low).map(format_price),
        ),
        (
            "PREV",
            snapshot
                .and_then(|quote| quote.previous_close)
                .map(format_price),
        ),
        (
            "VOLUME",
            snapshot.and_then(|quote| quote.volume).map(format_compact),
        ),
        ("EST. CAP", detail.company.market_cap.map(format_money)),
        ("SECTOR", detail.sector_return.map(format_percent)),
    ];
    let lines: Vec<Line<'_>> = rows
        .into_iter()
        .map(|(label, value)| {
            Line::from(vec![
                Span::styled(format!(" {label:<11}"), Style::default().fg(MUTED)),
                Span::styled(
                    value.unwrap_or_else(|| "--".to_owned()),
                    Style::default().fg(TEXT),
                ),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(tint))
            .block(
                Block::default()
                    .title(" STATISTICS ")
                    .borders(Borders::BOTTOM)
                    .border_style(BORDER),
            ),
        area,
    );
}

fn range_adds_no_history(detail: &TickerDetail, range: DateRange) -> bool {
    if range == DateRange::All {
        return false;
    }
    let Some(start) = detail.history_start_at else {
        return true;
    };
    if range == DateRange::Day {
        return false;
    }
    start >= range.previous().cutoff(detail.range_end_at)
}

fn compact_history_coverage(detail: &TickerDetail) -> Option<String> {
    let (start, end) = detail.history_start_at.zip(detail.history_end_at)?;
    Some(format!(
        "Data {} since {}",
        format_history_span(start, end),
        start.date_naive()
    ))
}

fn format_history_span(start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let total_days = end
        .signed_duration_since(start)
        .num_days()
        .max(0)
        .unsigned_abs();
    let years = total_days / 365;
    let after_years = total_days % 365;
    let months = after_years / 30;
    let days = after_years % 30;
    let mut parts = Vec::with_capacity(3);
    if years > 0 {
        parts.push(format!("{years}Y"));
    }
    if months > 0 {
        parts.push(format!("{months}M"));
    }
    if days > 0 || parts.is_empty() {
        parts.push(format!("{days}D"));
    }
    parts.join(" ")
}

fn render_description(frame: &mut Frame<'_>, detail: &TickerDetail, area: Rect, tint: Color) {
    let description = if detail.company.description.trim().is_empty() {
        let name = detail.company.name.trim();
        let symbol = detail.company.symbol.trim();
        let exchange = detail.company.exchange.trim();
        let industry = detail.company.industry.trim();
        match (exchange.is_empty(), industry.is_empty()) {
            (false, false) => format!(
                "{name} operates in the {} and trades on {exchange} as {symbol}.",
                industry_phrase(industry)
            ),
            (false, true) => format!("{name} trades on {exchange} as {symbol}."),
            (true, false) => {
                format!("{name} operates in the {}.", industry_phrase(industry))
            }
            (true, true) => name.to_owned(),
        }
    } else {
        detail.company.description.trim().to_owned()
    };
    frame.render_widget(
        Paragraph::new(description)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(MUTED).bg(tint))
            .block(
                Block::default()
                    .title(" COMPANY ")
                    .borders(Borders::TOP)
                    .border_style(BORDER),
            ),
        area,
    );
}

fn industry_phrase(industry: &str) -> String {
    let industry = industry.trim().trim_end_matches('.');
    if industry.to_ascii_lowercase().ends_with("industry") {
        industry.to_owned()
    } else {
        format!("{industry} industry")
    }
}

fn render_news(
    frame: &mut Frame<'_>,
    state: &mut UiState,
    news: &[NewsItem],
    area: Rect,
    tint: Color,
) {
    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    frame.render_widget(
        Block::default()
            .title(" NEWS ")
            .borders(Borders::ALL)
            .border_style(BORDER)
            .style(Style::default().bg(tint)),
        area,
    );
    if news.is_empty() {
        state.selected_news = 0;
        frame.render_widget(
            Paragraph::new("No cached headlines").style(Style::default().fg(MUTED).bg(tint)),
            inner,
        );
        return;
    }
    if inner.width < 2 {
        return;
    }
    state.selected_news = state.selected_news.min(news.len() - 1);
    const MAX_HEADLINE_LINES: u16 = 3;
    let headline_width = inner.width - 1;
    let row_heights = news
        .iter()
        .map(|item| {
            let headline = news_headline(item, false, tint);
            u16::try_from(headline.line_count(headline_width))
                .unwrap_or(u16::MAX)
                .clamp(1, MAX_HEADLINE_LINES)
                + 1
        })
        .collect::<Vec<_>>();
    let first_visible = news_view_start(&row_heights, state.selected_news, inner.height);
    let mut y = inner.y;
    for (index, item) in news.iter().enumerate().skip(first_visible) {
        if inner.bottom().saturating_sub(y) < 2 {
            break;
        }
        let selected = index == state.selected_news;
        let row_tint = if selected { PANEL_ALT } else { tint };
        let headline = news_headline(item, selected, row_tint);
        let row_height = row_heights[index];
        let headline_height = row_height - 1;
        if row_height > inner.bottom() - y {
            break;
        }
        let rect = Rect::new(inner.x, y, inner.width, row_height);
        let marker_rect = Rect::new(inner.x, y, 1, headline_height);
        let headline_rect = Rect::new(inner.x + 1, y, headline_width, headline_height);
        let metadata_rect = Rect::new(inner.x, y + headline_height, inner.width, 1);
        let published = item.published_at.with_timezone(&Local).format("%b %d");
        let source = match item.source.trim() {
            "" => "Source unavailable",
            source => source,
        };
        frame.render_widget(
            Paragraph::new(if selected { "›" } else { " " }).style(
                Style::default()
                    .fg(if selected { CYAN } else { TEXT })
                    .bg(row_tint)
                    .bold(),
            ),
            marker_rect,
        );
        frame.render_widget(headline, headline_rect);
        frame.render_widget(
            Paragraph::new(format!(" {published}  ·  {source}"))
                .style(Style::default().fg(MUTED).bg(row_tint)),
            metadata_rect,
        );
        state.register(rect, UiAction::OpenNews(index), None);
        y += row_height;
    }
}

fn news_headline(item: &NewsItem, selected: bool, tint: Color) -> Paragraph<'static> {
    Paragraph::new(Line::styled(
        item.headline.trim().to_owned(),
        Style::default()
            .fg(if selected { CYAN } else { TEXT })
            .bold(),
    ))
    .wrap(Wrap { trim: true })
    .style(Style::default().bg(tint))
}

fn news_view_start(row_heights: &[u16], selected: usize, available: u16) -> usize {
    let selected = selected.min(row_heights.len().saturating_sub(1));
    let mut start = 0;
    let mut used = 0u16;
    for (index, height) in row_heights.iter().copied().enumerate() {
        if index > start && used.saturating_add(height) > available {
            start = index;
            used = 0;
        }
        if index == selected {
            return start;
        }
        used = used.saturating_add(height);
    }
    start
}

fn render_overlay(frame: &mut Frame<'_>, state: &mut UiState, area: Rect, overlay: Overlay) {
    state.register(area, UiAction::CloseOverlay, None);
    match overlay {
        Overlay::Search => render_search(frame, state, area),
        Overlay::Sort => render_sort(frame, state, area),
        Overlay::Help => render_about(frame, state, area),
        Overlay::Sync => render_sync(frame, state, area),
    }
}

fn render_search(frame: &mut Frame<'_>, state: &mut UiState, area: Rect) {
    let height = (state.search_results.len() as u16 + 4).clamp(7, area.height.saturating_sub(4));
    let modal = centered(area, area.width.saturating_sub(8).min(84), height);
    frame.render_widget(Clear, modal);
    frame.render_widget(
        Block::default()
            .title(" COMPANY SEARCH ")
            .borders(Borders::ALL)
            .border_style(CYAN)
            .style(Style::default().bg(PANEL)),
        modal,
    );
    let inner = modal.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let query_rect = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(
        Paragraph::new(format!("⌕ {}", state.search_query))
            .style(Style::default().fg(TEXT).bg(PANEL_ALT)),
        query_rect,
    );
    let cursor_x = query_rect.x
        + 2
        + state
            .search_query
            .width()
            .min(query_rect.width as usize - 2) as u16;
    frame.set_cursor_position((cursor_x, query_rect.y));
    let mut result_targets = Vec::new();
    for (index, company) in state.search_results.iter().enumerate() {
        let y = inner.y + 2 + index as u16;
        if y >= inner.bottom() {
            break;
        }
        let rect = Rect::new(inner.x, y, inner.width, 1);
        let selected = index == state.search_selected;
        let style = if selected {
            Style::default().fg(CANVAS).bg(CYAN).bold()
        } else {
            Style::default().fg(TEXT).bg(PANEL)
        };
        let sector = company.sector.map_or("--", |sector| sector.label());
        let line = format!(
            " {:<7} {:<34} {:<12} {}",
            company.symbol, company.name, sector, company.exchange
        );
        frame.render_widget(Paragraph::new(line).style(style), rect);
        result_targets.push((rect, company.symbol.clone()));
    }
    for (rect, symbol) in result_targets {
        state.register(rect, UiAction::SearchResult(symbol.clone()), Some(symbol));
    }
}

fn render_sort(frame: &mut Frame<'_>, state: &mut UiState, area: Rect) {
    let modal = centered(area, 38.min(area.width.saturating_sub(4)), 8);
    frame.render_widget(Clear, modal);
    frame.render_widget(
        Block::default()
            .title(" ORDER TICKERS ")
            .borders(Borders::ALL)
            .border_style(CYAN)
            .style(Style::default().bg(PANEL)),
        modal,
    );
    for (index, mode) in SortMode::ALL.into_iter().enumerate() {
        let rect = Rect::new(modal.x + 1, modal.y + 2 + index as u16, modal.width - 2, 1);
        let selected = mode == state.sort;
        let style = if selected {
            Style::default().fg(CANVAS).bg(CYAN).bold()
        } else {
            Style::default().fg(TEXT).bg(PANEL)
        };
        frame.render_widget(
            Paragraph::new(format!(" {:<16}", mode.label())).style(style),
            rect,
        );
        state.register(rect, UiAction::SelectSort(mode), None);
    }
}

fn render_about(frame: &mut Frame<'_>, state: &mut UiState, area: Rect) {
    let modal = centered(area, 62.min(area.width.saturating_sub(4)), 23);
    frame.render_widget(Clear, modal);
    frame.render_widget(
        Block::default()
            .title(" HELP ")
            .borders(Borders::ALL)
            .border_style(CYAN)
            .style(Style::default().bg(PANEL)),
        modal,
    );
    let inner = modal.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let mut content = vec![
        help_row("Navigate", "arrows or h j k l"),
        help_row("Open", "Enter"),
        help_row("Back", "Esc"),
        help_row("Search", "/"),
        help_row("Sort", "s"),
        help_row("Metric", "i  Sector, Starred"),
        help_row("Order", "o  Overview, Sector, Starred"),
        help_row("View", "v  Overview, Sector, Starred"),
        help_row("Star", "f"),
        help_row("Starred", "F"),
        help_row("Refresh", "r"),
        help_row("Data status", "S"),
        help_row("Ranges", "1..9/0, [ ], =/+ in, - out"),
        help_row("Sectors", "g then c s h e t f i m u"),
        help_row("Prev / next", "Backspace / Space"),
        help_row("Detail tabs", "Tab"),
        help_row("Detail", "Left/Right chart, Up/Down news"),
        help_row("Quit", "q"),
    ];
    if inner.height >= 19 {
        content.insert(
            0,
            Line::styled("Keyboard", Style::default().fg(CYAN).bold()),
        );
    }
    if inner.height >= 20 {
        content.push(Line::styled(
            format!("Market prices and news: {}", state.data_provider_label),
            Style::default().fg(MUTED),
        ));
    }
    frame.render_widget(
        Paragraph::new(content).style(Style::default().fg(TEXT).bg(PANEL)),
        inner,
    );
}

fn help_row<'a>(label: &'a str, keys: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{label:<13}"), Style::default().fg(TEXT)),
        Span::styled(keys, Style::default().fg(MUTED)),
    ])
}

fn render_sync(frame: &mut Frame<'_>, state: &mut UiState, area: Rect) {
    let modal = centered(area, 62.min(area.width.saturating_sub(4)), 13);
    frame.render_widget(Clear, modal);
    let percent = (state.sync.fraction() * 100.0).round();
    let error = state.sync.last_error.as_deref().unwrap_or("None");
    let cadence = state.auto_refresh_interval.map_or_else(
        || "Disabled (demo/offline)".to_owned(),
        |interval| format!("Every {}", compact_duration(interval)),
    );
    let snapshot = state.snapshot_checkpoint.map_or_else(
        || "Not cached".to_owned(),
        |time| {
            time.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        },
    );
    let content = vec![
        status_row("Phase", format!("{:?}", state.sync.phase), TEXT),
        status_row(
            "Progress",
            format!(
                "{}/{} ({percent:.0}%)",
                state.sync.completed, state.sync.total
            ),
            TEXT,
        ),
        status_row("Status", state.sync.message.clone(), TEXT),
        status_row("Auto refresh", cadence, TEXT),
        status_row("Price cache", snapshot, TEXT),
        status_row(
            "Auto scope",
            "Active set + benchmarks + favorites".to_owned(),
            MUTED,
        ),
        status_row(
            "Manual r",
            "Retained candidates + incremental history".to_owned(),
            MUTED,
        ),
        status_row(
            "Stale data",
            "Provider observation is over 72h old".to_owned(),
            MUTED,
        ),
        status_row("Last error", error.to_owned(), MUTED),
    ];
    frame.render_widget(
        Block::default()
            .title(" DATA STATUS ")
            .borders(Borders::ALL)
            .border_style(CYAN)
            .style(Style::default().bg(PANEL)),
        modal,
    );
    let inner = modal.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    frame.render_widget(
        Paragraph::new(content).style(Style::default().fg(TEXT).bg(PANEL)),
        inner,
    );
}

fn status_row(label: &str, value: String, value_color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<13}"), Style::default().fg(MUTED)),
        Span::styled(value, Style::default().fg(value_color)),
    ])
}

fn compact_duration(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    if seconds.is_multiple_of(3_600) {
        format!("{}h", seconds / 3_600)
    } else if seconds.is_multiple_of(60) {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width.min(area.width),
        height.min(area.height),
    )
}

fn performance_accent(value: Option<f64>) -> Color {
    if value.unwrap_or(0.0) >= 0.0 {
        Color::Rgb(98, 232, 93)
    } else {
        Color::Rgb(255, 79, 68)
    }
}

fn format_price(value: f64) -> String {
    format!("${value:.2}")
}

fn format_signed_price(value: f64) -> String {
    let sign = if value.is_sign_negative() { '-' } else { '+' };
    format!("{sign}${:.2}", value.abs())
}

fn format_percent(value: f64) -> String {
    format!("{:+.2}%", value * 100.0)
}

fn format_money(value: f64) -> String {
    format!("${}", format_compact(value))
}

fn format_compact(value: f64) -> String {
    let (scaled, suffix) = if value.abs() >= 1_000_000_000_000.0 {
        (value / 1_000_000_000_000.0, "T")
    } else if value.abs() >= 1_000_000_000.0 {
        (value / 1_000_000_000.0, "B")
    } else if value.abs() >= 1_000_000.0 {
        (value / 1_000_000.0, "M")
    } else if value.abs() >= 1_000.0 {
        (value / 1_000.0, "K")
    } else {
        (value, "")
    };
    format!("{scaled:.2}{suffix}")
}

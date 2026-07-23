use chrono::Local;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    domain::{DateRange, NewsItem, SortMode, TickerDetail},
    palette::{AMBER, BORDER, CANVAS, CYAN, MUTED, PANEL, PANEL_ALT, TEXT, detail_tint},
    ui::{
        chart::render_price_volume,
        heatmap,
        layout::{AppLayout, LayoutMode},
        state::{DetailTab, Overlay, Route, UiAction, UiState},
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
    render_footer(frame, state, layout.footer);
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
    let right = format!("{}  ·  {} ", state.date_range, state.sort.label());
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
    let inspector_symbol = match state.route {
        Route::Sector(_) | Route::Favorites => state.focused_symbol(),
        Route::Overview | Route::Ticker(_) => state.hovered_symbol.as_deref(),
    };
    let inspector = inspector_symbol
        .and_then(|symbol| {
            state
                .tiles
                .iter()
                .find(|tile| tile.company.symbol == symbol)
        })
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
    frame.render_widget(
        Paragraph::new(format!(" {inspector}")).style(Style::default().fg(MUTED).bg(PANEL_ALT)),
        Rect::new(area.x, area.y + 1, area.width, 1),
    );
}

fn render_rail(frame: &mut Frame<'_>, state: &mut UiState, area: Rect) {
    frame
        .buffer_mut()
        .set_style(area, Style::default().bg(PANEL));
    let mut y = area.y;
    if !matches!(state.route, Route::Overview) || state.overlay.is_some() {
        y = rail_button(frame, state, area, y, "Esc", "Back", UiAction::Back, false);
    }
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
    if let Some(symbol) = state.focused_symbol().map(str::to_owned) {
        let starred = state
            .tiles
            .iter()
            .find(|tile| tile.company.symbol == symbol)
            .is_some_and(|tile| tile.starred);
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
    for (index, range) in DateRange::ALL.into_iter().enumerate() {
        if y >= area.bottom().saturating_sub(3) {
            break;
        }
        let label = format!("{}: {}", index + 1, range.label());
        y = rail_button(
            frame,
            state,
            area,
            y,
            "",
            &label,
            UiAction::SelectRange(range),
            state.date_range == range,
        );
    }
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

fn render_footer(frame: &mut Frame<'_>, state: &UiState, area: Rect) {
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
    let left_width = area.width.saturating_sub(right.width() as u16);
    frame.render_widget(
        Paragraph::new(format!(" {}", state.sync.message))
            .style(Style::default().fg(MUTED).bg(PANEL_ALT)),
        Rect::new(area.x, area.y, left_width, 1),
    );
    frame.render_widget(
        Paragraph::new(right)
            .alignment(Alignment::Right)
            .style(Style::default().fg(MUTED).bg(PANEL_ALT)),
        Rect::new(area.x + left_width, area.y, area.width - left_width, 1),
    );
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
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(5)])
        .split(columns[0]);
    let accent = performance_accent(detail.period_return);
    render_price_volume(frame, state, left[0], &detail.bars, accent);
    render_description(frame, detail, left[1], tint);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(10), Constraint::Min(6)])
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
    let price = detail
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.price)
        .map_or_else(|| "--".to_owned(), |value| format!("${value:.2}"));
    let period_return = detail.period_return.map_or_else(
        || "--".to_owned(),
        |value| format!("{:+.2}%", value * 100.0),
    );
    let rank = detail
        .sector_rank
        .map_or_else(|| "--".to_owned(), |value| format!("#{value}"));
    let sector = detail
        .company
        .sector
        .map_or("Unclassified", |sector| sector.label());
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
                format!("  {price}  {period_return}"),
                Style::default().fg(TEXT).bold(),
            ),
        ]),
        Line::styled(
            format!(" {}", detail.company.name),
            Style::default().fg(TEXT),
        ),
        Line::styled(
            format!(" {sector}  ·  {rank} in sector"),
            Style::default().fg(MUTED),
        ),
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
        ("MARKET CAP", detail.company.market_cap.map(format_money)),
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

fn render_description(frame: &mut Frame<'_>, detail: &TickerDetail, area: Rect, tint: Color) {
    let description = if detail.company.description.is_empty() {
        format!("{} · {}", detail.company.name, detail.company.industry)
    } else {
        detail.company.description.clone()
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
    state.selected_news = state.selected_news.min(news.len() - 1);
    let row_height = if inner.height >= 12 { 3 } else { 2 };
    for (index, item) in news.iter().enumerate() {
        let y = inner.y + index as u16 * row_height;
        if y >= inner.bottom() {
            break;
        }
        let height = row_height.min(inner.bottom() - y);
        let rect = Rect::new(inner.x, y, inner.width, height);
        let published = item.published_at.with_timezone(&Local).format("%b %d");
        let selected = index == state.selected_news;
        let row_tint = if selected { PANEL_ALT } else { tint };
        let marker = if selected { "›" } else { " " };
        let text = vec![
            Line::styled(
                format!("{marker}{}", item.headline),
                Style::default()
                    .fg(if selected { CYAN } else { TEXT })
                    .bold(),
            ),
            Line::styled(
                format!(" {published}  ·  {}", item.source),
                Style::default().fg(MUTED),
            ),
        ];
        frame.render_widget(
            Paragraph::new(text)
                .wrap(Wrap { trim: true })
                .style(Style::default().bg(row_tint)),
            rect,
        );
        state.register(rect, UiAction::OpenNews(index), None);
    }
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

fn render_about(frame: &mut Frame<'_>, _state: &mut UiState, area: Rect) {
    let modal = centered(area, 58.min(area.width.saturating_sub(4)), 17);
    frame.render_widget(Clear, modal);
    let content = vec![
        Line::styled("Keyboard", Style::default().fg(CYAN).bold()),
        Line::from("Navigate     arrows or h j k l"),
        Line::from("Open         Enter"),
        Line::from("Back         Esc or Backspace"),
        Line::from("Search       /"),
        Line::from("Sort         s"),
        Line::from("Star         f"),
        Line::from("Starred      F"),
        Line::from("Refresh      r"),
        Line::from("Data status  S"),
        Line::from("Ranges       1..7 or [ ]"),
        Line::from("Detail tabs  Tab"),
        Line::from("Quit         q"),
        Line::from(""),
        Line::styled("Market prices and news: Alpaca", Style::default().fg(MUTED)),
    ];
    frame.render_widget(
        Paragraph::new(content)
            .centered()
            .style(Style::default().fg(TEXT).bg(PANEL))
            .block(
                Block::default()
                    .title(" HELP ")
                    .borders(Borders::ALL)
                    .border_style(CYAN),
            ),
        modal,
    );
}

fn render_sync(frame: &mut Frame<'_>, state: &mut UiState, area: Rect) {
    let modal = centered(area, 62.min(area.width.saturating_sub(4)), 11);
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
        Line::from(format!("Phase       {:?}", state.sync.phase)),
        Line::from(format!(
            "Progress    {}/{} ({percent:.0}%)",
            state.sync.completed, state.sync.total
        )),
        Line::from(format!("Status      {}", state.sync.message)),
        Line::from(format!("Auto refresh {cadence}")),
        Line::from(format!("Price cache {snapshot}")),
        Line::styled(format!("Last error  {error}"), Style::default().fg(MUTED)),
    ];
    frame.render_widget(
        Paragraph::new(content)
            .style(Style::default().fg(TEXT).bg(PANEL))
            .block(
                Block::default()
                    .title(" DATA STATUS ")
                    .borders(Borders::ALL)
                    .border_style(CYAN),
            ),
        modal,
    );
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

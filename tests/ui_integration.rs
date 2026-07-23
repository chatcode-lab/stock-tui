use chrono::{Duration, Local, TimeZone, Utc};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, layout::Rect, style::Modifier};
use stock_tui::{
    app::{AppCommand, handle_event},
    benchmarks::MarketBenchmark,
    domain::{Bar, Company, DateRange, MarketTile, NewsItem, Sector, Snapshot, TickerDetail},
    palette::{CANVAS, CYAN, HeatScale, MUTED, PANEL, Theme},
    ui::{
        layout::AppLayout,
        render,
        state::{DetailTab, Overlay, Route, UiAction, UiState},
    },
};

const VIEWPORTS: [(u16, u16); 5] = [(60, 20), (80, 24), (120, 40), (160, 48), (200, 60)];

#[test]
fn overview_renders_at_supported_viewports_with_visible_hit_targets() {
    for (width, height) in VIEWPORTS {
        let mut state = fixture_state();
        let buffer = render_at(&mut state, width, height);
        let screen = screen_text(&buffer);

        assert_eq!(buffer.area, Rect::new(0, 0, width, height));
        assert!(
            screen.contains("STOCK TUI"),
            "missing header at {width}x{height}"
        );
        assert!(
            screen.contains("Consumer"),
            "missing heatmap content at {width}x{height}"
        );
        assert!(
            !screen.contains("needs at least"),
            "supported viewport was rejected at {width}x{height}"
        );
        assert!(
            state.hit_targets.len() >= 20,
            "too few hit targets at {width}x{height}: {}",
            state.hit_targets.len()
        );

        for target in &state.hit_targets {
            assert!(target.rect.width > 0 && target.rect.height > 0);
            assert!(target.rect.right() <= width && target.rect.bottom() <= height);
            assert!(
                rect_has_visible_cell(&buffer, target.rect),
                "blank target {:?} at {width}x{height}",
                target.action
            );
        }

        if width < 120 || height < 36 {
            assert!(
                buffer.content().iter().any(|cell| cell.symbol() == "▀"),
                "compact overview did not use paired heatmap rows at {width}x{height}"
            );
        }
    }
}

#[test]
fn keyboard_drills_from_overview_through_sector_to_ticker() {
    let mut state = fixture_state();
    state.selected_sector = Sector::ALL
        .iter()
        .position(|sector| *sector == Sector::Technology)
        .expect("technology is in the fixed sector list");

    let commands = press(&mut state, KeyCode::Enter, KeyModifiers::NONE);
    assert!(commands.is_empty());
    assert_eq!(state.route, Route::Sector(Sector::Technology));

    let buffer = render_at(&mut state, 80, 24);
    let screen = screen_text(&buffer);
    assert!(screen.contains("TECHNOLOGY / TOP 100"));
    assert!(screen.contains("ACME"));
    assert!(
        state
            .hit_targets
            .iter()
            .any(|target| target.action == UiAction::OpenTicker("ACME".to_owned()))
    );

    let commands = press(&mut state, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(commands, vec![AppCommand::LoadTicker("ACME".to_owned())]);
    assert_eq!(state.route, Route::Ticker("ACME".to_owned()));
}

#[test]
fn detail_renders_combined_full_view_and_each_compact_tab() {
    let mut full = detail_state();
    let full_buffer = render_at(&mut full, 160, 48);
    let full_screen = screen_text(&full_buffer);

    for expected in [
        "ACME / DETAIL",
        "$103.75",
        "+$7.01",
        "+7.25%",
        "PRICE",
        "VOLUME",
        "STATISTICS",
        "MARKET CAP",
        "NEWS",
        "Acme expands terminal analytics coverage",
    ] {
        assert!(
            full_screen.contains(expected),
            "missing {expected:?} in full detail"
        );
    }
    assert!(full.chart_rect.is_some());
    assert!(!full.chart_sample_indices.is_empty());
    assert!(full_screen.contains('★'));
    assert!(full.hit_targets.iter().any(|target| {
        target.action == UiAction::ToggleFavorite("ACME".to_owned()) && target.rect.width == 1
    }));
    assert!(
        full.hit_targets
            .iter()
            .any(|target| target.action == UiAction::OpenNews(0))
    );

    let mut unstarred = detail_state();
    unstarred.detail.as_mut().expect("fixture detail").starred = false;
    assert!(screen_text(&render_at(&mut unstarred, 160, 48)).contains('☆'));

    let mut compact = detail_state();
    let chart_buffer = render_at(&mut compact, 80, 24);
    let chart_screen = screen_text(&chart_buffer);
    assert!(chart_screen.contains("Chart"));
    assert!(chart_screen.contains("PRICE"));
    assert!(chart_screen.contains("VOLUME"));
    assert!(compact.chart_rect.is_some());

    assert!(press(&mut compact, KeyCode::Tab, KeyModifiers::NONE).is_empty());
    assert_eq!(compact.detail_tab, DetailTab::Statistics);
    let statistics_screen = screen_text(&render_at(&mut compact, 80, 24));
    assert!(statistics_screen.contains("STATISTICS"));
    assert!(statistics_screen.contains("OPEN"));
    assert!(statistics_screen.contains("MARKET CAP"));

    assert!(press(&mut compact, KeyCode::Tab, KeyModifiers::NONE).is_empty());
    assert_eq!(compact.detail_tab, DetailTab::News);
    let news_buffer = render_at(&mut compact, 80, 24);
    let news_screen = screen_text(&news_buffer);
    assert!(news_screen.contains("NEWS"));
    assert!(news_screen.contains("Acme expands terminal analytics coverage"));
    assert!(
        compact
            .hit_targets
            .iter()
            .any(|target| target.action == UiAction::OpenNews(0))
    );

    let second_news = compact
        .hit_targets
        .iter()
        .find(|target| target.action == UiAction::OpenNews(1))
        .cloned()
        .expect("second news row has a mouse target");
    assert!(
        handle_event(
            &mut compact,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: second_news.rect.x,
                row: second_news.rect.y,
                modifiers: KeyModifiers::NONE,
            }),
        )
        .is_empty()
    );
    assert_eq!(compact.selected_news, 1);

    assert!(press(&mut compact, KeyCode::Up, KeyModifiers::NONE).is_empty());
    assert_eq!(compact.selected_news, 0);
    assert!(press(&mut compact, KeyCode::Down, KeyModifiers::NONE).is_empty());
    assert_eq!(
        press(&mut compact, KeyCode::Enter, KeyModifiers::NONE),
        vec![AppCommand::OpenUrl(
            "https://example.invalid/acme/results".to_owned()
        )]
    );
}

#[test]
fn detail_arrow_axes_keep_chart_and_news_selection_independent() {
    let mut state = detail_state();
    render_at(&mut state, 160, 48);
    let plot = state
        .chart_rect
        .expect("detail chart exposes its plot area");
    let hovered_index = 5;

    assert!(
        handle_event(
            &mut state,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: plot.x + hovered_index,
                row: plot.y + plot.height / 2,
                modifiers: KeyModifiers::NONE,
            }),
        )
        .is_empty()
    );
    assert_eq!(state.detail_hover, Some(usize::from(hovered_index)));

    state.selected_news = 1;
    assert!(press(&mut state, KeyCode::Up, KeyModifiers::NONE).is_empty());
    assert_eq!(state.selected_news, 0);
    assert_eq!(state.detail_hover, Some(usize::from(hovered_index)));

    assert!(press(&mut state, KeyCode::Right, KeyModifiers::NONE).is_empty());
    assert_eq!(state.detail_hover, Some(usize::from(hovered_index) + 1));
    assert_eq!(state.selected_news, 0);
    assert_eq!(
        press(&mut state, KeyCode::Enter, KeyModifiers::NONE),
        vec![AppCommand::OpenUrl(
            "https://example.invalid/acme/analytics".to_owned()
        )]
    );

    state.detail_tab = DetailTab::News;
    assert!(press(&mut state, KeyCode::Left, KeyModifiers::NONE).is_empty());
    assert_eq!(state.detail_hover, Some(usize::from(hovered_index)));
    assert!(press(&mut state, KeyCode::Down, KeyModifiers::NONE).is_empty());
    assert_eq!(state.selected_news, 1);
}

#[test]
fn detail_chart_fills_the_plot_and_exposes_aligned_axes_and_hover() {
    let mut state = detail_state();
    let buffer = render_at(&mut state, 160, 48);
    let plot = state
        .chart_rect
        .expect("detail chart exposes its plot area");

    let gradient_cells = plot
        .rows()
        .flat_map(|row| row.columns())
        .filter(|position| buffer[*position].bg != PANEL)
        .count();
    assert!(
        gradient_cells > usize::from(plot.width),
        "area chart should shade multiple rows beneath the price trace"
    );

    let braille_cells = plot
        .rows()
        .flat_map(|row| row.columns())
        .filter(|position| {
            buffer[*position]
                .symbol()
                .chars()
                .next()
                .is_some_and(|symbol| ('\u{2801}'..='\u{28ff}').contains(&symbol))
        })
        .count();
    assert!(braille_cells > 0, "price trace should use Braille cells");

    let guide_cells = plot
        .rows()
        .flat_map(|row| row.columns())
        .filter(|position| buffer[*position].symbol() == "·")
        .count();
    assert!(
        guide_cells >= usize::from(plot.width),
        "reference guides should use terminal-stable middle dots"
    );

    let mut in_plot_axis = Vec::new();
    for y in plot.y..plot.bottom() {
        for x in plot.x..plot.right() {
            if buffer[(x, y)].symbol() == "$" {
                in_plot_axis.push((x, y));
            }
        }
    }
    assert!(
        !in_plot_axis.is_empty(),
        "price labels should overlay the plot"
    );
    assert!(
        in_plot_axis
            .iter()
            .all(|(x, _)| *x < plot.x.saturating_add(10)),
        "price labels should stay near the plot's left edge"
    );

    let x_axis: String = (plot.x..plot.right())
        .map(|x| buffer[(x, plot.bottom())].symbol())
        .collect();
    assert!(
        x_axis.contains(':'),
        "one-day time axis should show intraday labels"
    );

    let hover_commands = handle_event(
        &mut state,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: plot.right() - 1,
            row: plot.y + plot.height / 2,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert!(hover_commands.is_empty());
    assert_eq!(
        state.detail_hover,
        Some(state.chart_sample_indices.len() - 1)
    );

    let hovered = render_at(&mut state, 160, 48);
    let hovered_plot = state.chart_rect.expect("hovered chart keeps its plot area");
    let cursor_x = hovered_plot.right() - 1;
    let cursor_symbol = hovered[(cursor_x, hovered_plot.y)].symbol().to_owned();
    assert!(
        cursor_symbol
            .chars()
            .next()
            .is_some_and(|symbol| ('\u{2801}'..='\u{28ff}').contains(&symbol))
    );
    assert!(
        (hovered_plot.y..hovered_plot.bottom())
            .all(|y| hovered[(cursor_x, y)].symbol() == cursor_symbol),
        "hover cursor should use one fixed Braille subcolumn through grid and label rows"
    );
    assert_eq!(
        (hovered_plot.y..hovered_plot.bottom())
            .filter(|y| {
                let cell = &hovered[(cursor_x, *y)];
                cell.fg == CANVAS && cell.bg == CYAN
            })
            .count(),
        1,
        "exactly one high-contrast cursor cell should mark the selected price"
    );
}

#[test]
fn search_accepts_input_renders_results_and_opens_selection() {
    let mut state = fixture_state();
    state.search_results = state
        .tiles
        .iter()
        .filter(|tile| tile.company.sector == Some(Sector::Technology))
        .map(|tile| tile.company.clone())
        .collect();

    assert_eq!(
        press(&mut state, KeyCode::Char('/'), KeyModifiers::NONE),
        vec![AppCommand::Search(String::new())]
    );
    assert_eq!(state.overlay, Some(Overlay::Search));
    assert_eq!(
        press(&mut state, KeyCode::Char('a'), KeyModifiers::NONE),
        vec![AppCommand::Search("a".to_owned())]
    );

    let buffer = render_at(&mut state, 80, 24);
    let screen = screen_text(&buffer);
    assert!(screen.contains("COMPANY SEARCH"));
    assert!(screen.contains('⌕'));
    assert_eq!(state.search_query, "a");
    assert!(screen.contains("ACME"));
    assert!(screen.contains("BETA"));
    assert!(
        state
            .hit_targets
            .iter()
            .any(|target| target.action == UiAction::SearchResult("ACME".to_owned()))
    );

    assert!(press(&mut state, KeyCode::Down, KeyModifiers::NONE).is_empty());
    assert_eq!(state.search_selected, 1);
    assert_eq!(
        press(&mut state, KeyCode::Enter, KeyModifiers::NONE),
        vec![AppCommand::LoadTicker("BETA".to_owned())]
    );
    assert_eq!(state.overlay, None);
    assert_eq!(state.route, Route::Ticker("BETA".to_owned()));
}

#[test]
fn rendered_mouse_target_hovers_and_opens_ticker() {
    let mut state = fixture_state();
    state.route = Route::Sector(Sector::Technology);
    render_at(&mut state, 80, 24);

    let target = state
        .hit_targets
        .iter()
        .find(|target| target.action == UiAction::OpenTicker("BETA".to_owned()))
        .cloned()
        .expect("sector render registers the BETA tile");
    let column = target.rect.x + target.rect.width / 2;
    let row = target.rect.y + target.rect.height / 2;

    let hover_commands = handle_event(
        &mut state,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert!(hover_commands.is_empty());
    assert_eq!(state.hovered_symbol.as_deref(), Some("BETA"));
    assert_eq!(state.selected_ticker, 1);
    assert_eq!(state.focused_symbol(), Some("BETA"));

    assert!(press(&mut state, KeyCode::Left, KeyModifiers::NONE).is_empty());
    assert_eq!(state.hovered_symbol, None);
    assert_eq!(state.selected_ticker, 0);
    assert_eq!(state.focused_symbol(), Some("ACME"));
    assert!(
        screen_text(&render_at(&mut state, 80, 24)).contains("ACME  Acme Systems  $108.00  -0.50%"),
        "keyboard ticker selection should update the header inspector"
    );
    assert!(
        handle_event(
            &mut state,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            }),
        )
        .is_empty()
    );

    let leave_commands = handle_event(
        &mut state,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert!(leave_commands.is_empty());
    assert_eq!(state.hovered_symbol, None);
    assert_eq!(state.selected_ticker, 1);
    assert_eq!(state.focused_symbol(), Some("BETA"));

    let click_commands = handle_event(
        &mut state,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert_eq!(
        click_commands,
        vec![AppCommand::LoadTicker("BETA".to_owned())]
    );
    assert_eq!(state.route, Route::Ticker("BETA".to_owned()));

    assert!(press(&mut state, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    assert_eq!(state.route, Route::Sector(Sector::Technology));
    assert_eq!(state.selected_ticker, 1);
    assert_eq!(state.focused_symbol(), Some("BETA"));
}

#[test]
fn bright_heat_tile_uses_dark_focus_contrast() {
    let mut state = fixture_state();
    state.theme = Theme::Default;
    state.route = Route::Sector(Sector::Utilities);
    render_at(&mut state, 80, 24);

    let target = state
        .hit_targets
        .iter()
        .find(|target| target.action == UiAction::OpenTicker("S81".to_owned()))
        .cloned()
        .expect("bright utility tile has a mouse target");
    assert!(
        handle_event(
            &mut state,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: target.rect.x + target.rect.width / 2,
                row: target.rect.y,
                modifiers: KeyModifiers::NONE,
            }),
        )
        .is_empty()
    );

    let buffer = render_at(&mut state, 80, 24);
    let focused = state
        .hit_targets
        .iter()
        .find(|target| target.action == UiAction::OpenTicker("S81".to_owned()))
        .expect("focused utility tile remains rendered");
    assert_eq!(buffer[(focused.rect.x, focused.rect.y)].fg, CANVAS);
}

#[test]
fn stale_bright_heat_tile_keeps_contrast_and_uses_an_underline_hint() {
    let mut state = fixture_state();
    state.theme = Theme::Default;
    state.route = Route::Sector(Sector::Utilities);
    let stale_tile = state
        .tiles
        .iter_mut()
        .find(|tile| tile.company.symbol == "S81")
        .expect("fixture contains the second utility tile");
    stale_tile.period_return = Some(1.0);
    stale_tile.stale = true;

    let scale = HeatScale::from_values(
        state.tiles.iter().map(|tile| tile.period_return),
        0.005,
        state.theme,
    );
    let buffer = render_at(&mut state, 80, 24);
    let target = state
        .hit_targets
        .iter()
        .find(|target| target.action == UiAction::OpenTicker("S81".to_owned()))
        .expect("stale utility tile remains rendered");
    let cell = &buffer[(target.rect.x, target.rect.y)];

    assert_eq!(cell.fg, scale.text_color(Some(1.0)));
    assert_ne!(cell.fg, MUTED);
    assert!(cell.modifier.contains(Modifier::UNDERLINED));
}

#[test]
fn overview_hover_selects_only_the_enclosing_sector() {
    let mut state = fixture_state();
    render_at(&mut state, 120, 40);

    let target = state
        .hit_targets
        .iter()
        .find(|target| target.action == UiAction::OpenSector(Sector::Technology))
        .cloned()
        .expect("overview render registers the technology panel");
    let hover_commands = handle_event(
        &mut state,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: target.rect.x + target.rect.width / 2,
            row: target.rect.y + target.rect.height / 2,
            modifiers: KeyModifiers::NONE,
        }),
    );

    assert!(hover_commands.is_empty());
    assert_eq!(
        state.selected_sector,
        Sector::ALL
            .iter()
            .position(|sector| *sector == Sector::Technology)
            .unwrap()
    );
    assert_eq!(state.hovered_symbol, None);
    assert_eq!(state.focused_symbol(), None);

    let buffer = render_at(&mut state, 120, 40);
    assert!(
        (target.rect.y..target.rect.bottom()).all(|y| buffer[(target.rect.x, y)].symbol() == "▌"),
        "selected sector should have one continuous panel-level marker"
    );
}

#[test]
fn overview_benchmark_footer_renders_selects_and_opens_equal_cells() {
    for (width, height) in VIEWPORTS {
        let mut state = fixture_state();
        render_at(&mut state, width, height);
        let layout = AppLayout::calculate(Rect::new(0, 0, width, height));
        let benchmark_targets: Vec<_> = state
            .hit_targets
            .iter()
            .filter(|target| {
                matches!(
                    &target.action,
                    UiAction::OpenTicker(symbol)
                        if MarketBenchmark::for_symbol(symbol).is_some()
                )
            })
            .collect();
        assert_eq!(benchmark_targets.len(), 3);
        for (index, benchmark) in benchmark_targets.iter().enumerate() {
            let sector = state
                .hit_targets
                .iter()
                .find(|target| target.action == UiAction::OpenSector(Sector::ALL[index + 6]))
                .expect("corresponding bottom-row sector target");
            assert_eq!(
                (benchmark.rect.x, benchmark.rect.width),
                (sector.rect.x, sector.rect.width),
                "benchmark column {index} does not align at {width}x{height}"
            );
            assert!(benchmark.rect.right() <= layout.content.right());
        }
        assert!(
            layout.content.right() < layout.footer.right(),
            "test viewport must reserve footer width for the action rail"
        );
    }

    let mut state = fixture_state();
    let buffer = render_at(&mut state, 120, 40);
    let screen = screen_text(&buffer);
    for expected in [
        "S&P 500 · SPY",
        "DOW · DIA",
        "NASDAQ 100 · QQQ",
        "$510.25",
        "-1.20%",
        "+2.10%",
    ] {
        assert!(
            screen.contains(expected),
            "missing benchmark footer value {expected:?}"
        );
    }

    let layout = AppLayout::calculate(Rect::new(0, 0, 120, 40));
    let footer = layout.footer;
    let benchmark_targets: Vec<_> = state
        .hit_targets
        .iter()
        .filter(|target| {
            matches!(
                &target.action,
                UiAction::OpenTicker(symbol)
                    if MarketBenchmark::for_symbol(symbol).is_some()
            )
        })
        .cloned()
        .collect();
    assert_eq!(benchmark_targets.len(), 3);
    assert!(
        benchmark_targets
            .iter()
            .all(|target| target.rect.y == footer.y
                && target.rect.height == 1
                && target.rect.width == benchmark_targets[0].rect.width)
    );
    assert!(
        benchmark_targets
            .iter()
            .all(|target| target.rect.right() <= layout.content.right())
    );

    let qqq = benchmark_targets
        .iter()
        .find(|target| target.action == UiAction::OpenTicker("QQQ".to_owned()))
        .expect("QQQ benchmark target");
    assert!(
        handle_event(
            &mut state,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: qqq.rect.x + qqq.rect.width / 2,
                row: qqq.rect.y,
                modifiers: KeyModifiers::NONE,
            }),
        )
        .is_empty()
    );
    assert_eq!(state.selected_benchmark, Some(2));

    let selected_buffer = render_at(&mut state, 120, 40);
    assert!((qqq.rect.x..qqq.rect.right()).any(|x| {
        selected_buffer[(x, qqq.rect.y)]
            .modifier
            .contains(Modifier::BOLD)
    }));
    let technology = state
        .hit_targets
        .iter()
        .find(|target| target.action == UiAction::OpenSector(Sector::Technology))
        .expect("technology sector target")
        .clone();
    assert!(
        handle_event(
            &mut state,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: technology.rect.x + technology.rect.width / 2,
                row: technology.rect.y + technology.rect.height / 2,
                modifiers: KeyModifiers::NONE,
            }),
        )
        .is_empty()
    );
    assert_eq!(state.selected_benchmark, None);
    assert!(
        handle_event(
            &mut state,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: qqq.rect.x + qqq.rect.width / 2,
                row: qqq.rect.y,
                modifiers: KeyModifiers::NONE,
            }),
        )
        .is_empty()
    );
    assert_eq!(state.selected_benchmark, Some(2));
    assert_eq!(
        handle_event(
            &mut state,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: qqq.rect.x + qqq.rect.width / 2,
                row: qqq.rect.y,
                modifiers: KeyModifiers::NONE,
            }),
        ),
        vec![AppCommand::LoadTicker("QQQ".to_owned())]
    );
    assert_eq!(state.route, Route::Ticker("QQQ".to_owned()));

    let mut compact = fixture_state();
    let compact_screen = screen_text(&render_at(&mut compact, 60, 20));
    for symbol in ["SPY", "DIA", "QQQ"] {
        assert!(
            compact_screen.contains(symbol),
            "compact benchmark footer hides {symbol}"
        );
    }
}

#[test]
fn sector_heatmap_uses_equal_centered_tiles_without_corner_artifacts() {
    let mut state = fixture_state();
    let template = state
        .tiles
        .iter()
        .find(|tile| tile.company.symbol == "ACME")
        .cloned()
        .expect("fixture contains a technology tile");
    for rank in 3..=100 {
        let mut tile = template.clone();
        tile.company.symbol = format!("T{rank:03}");
        tile.company.name = format!("Technology Fixture {rank}");
        tile.company.rank = Some(rank);
        tile.starred = false;
        state.tiles.push(tile);
    }
    state.route = Route::Sector(Sector::Technology);

    let width = 200;
    let height = 60;
    let buffer = render_at(&mut state, width, height);
    let content = AppLayout::calculate(Rect::new(0, 0, width, height)).content;
    let tiles: Vec<_> = state
        .hit_targets
        .iter()
        .filter(|target| matches!(target.action, UiAction::OpenTicker(_)))
        .collect();

    assert_eq!(tiles.len(), 100);
    let cell_size = (tiles[0].rect.width, tiles[0].rect.height);
    assert!(tiles.iter().all(|target| {
        (target.rect.width, target.rect.height) == cell_size
            && target.rect.width > 0
            && target.rect.height > 0
    }));

    let left = tiles.iter().map(|target| target.rect.x).min().unwrap();
    let right = tiles
        .iter()
        .map(|target| target.rect.right())
        .max()
        .unwrap();
    let top = tiles.iter().map(|target| target.rect.y).min().unwrap();
    let bottom = tiles
        .iter()
        .map(|target| target.rect.bottom())
        .max()
        .unwrap();
    assert!(left.abs_diff(content.x) <= content.right().abs_diff(right) + 1);
    assert!(content.right().abs_diff(right) <= left.abs_diff(content.x) + 1);
    assert!(top.abs_diff(content.y) <= content.bottom().abs_diff(bottom) + 1);
    assert!(content.bottom().abs_diff(bottom) <= top.abs_diff(content.y) + 1);

    for target in tiles {
        let left_cell = &buffer[(target.rect.x, target.rect.y)];
        let top_right = &buffer[(target.rect.right() - 1, target.rect.y)];
        assert_eq!(
            top_right.bg, left_cell.bg,
            "tile top-right corner uses a different background"
        );
    }
}

#[test]
fn keyboard_changes_ranges_opens_favorites_toggles_star_and_goes_back() {
    let mut state = fixture_state();
    state.route = Route::Sector(Sector::Technology);

    assert_eq!(
        press(&mut state, KeyCode::Char('f'), KeyModifiers::NONE),
        vec![AppCommand::ToggleFavorite("ACME".to_owned())]
    );
    assert_eq!(
        press(&mut state, KeyCode::Char('8'), KeyModifiers::NONE),
        vec![AppCommand::ReloadTiles]
    );
    assert_eq!(state.date_range, DateRange::FiveYears);
    assert_eq!(
        press(&mut state, KeyCode::Char('['), KeyModifiers::NONE),
        vec![AppCommand::ReloadTiles]
    );
    assert_eq!(state.date_range, DateRange::TwoYears);
    assert_eq!(
        press(&mut state, KeyCode::Char('0'), KeyModifiers::NONE),
        vec![AppCommand::ReloadTiles]
    );
    assert_eq!(state.date_range, DateRange::All);
    assert_eq!(
        press(&mut state, KeyCode::Char('['), KeyModifiers::NONE),
        vec![AppCommand::ReloadTiles]
    );
    assert_eq!(state.date_range, DateRange::TenYears);

    assert!(press(&mut state, KeyCode::Char('F'), KeyModifiers::SHIFT).is_empty());
    assert_eq!(state.route, Route::Favorites);
    assert!(press(&mut state, KeyCode::Backspace, KeyModifiers::NONE).is_empty());
    assert_eq!(state.route, Route::Overview);

    state.route = Route::Ticker("ACME".to_owned());
    state.detail = Some(fixture_detail());
    assert!(press(&mut state, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    assert_eq!(state.route, Route::Sector(Sector::Technology));
    assert!(state.detail.is_none());
    assert!(press(&mut state, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    assert_eq!(state.route, Route::Overview);
}

#[test]
fn rail_and_help_expose_keyboard_controls_and_demo_state() {
    let mut state = fixture_state();
    state.simulated_data = true;

    let screen = screen_text(&render_at(&mut state, 80, 24));
    for expected in [
        "SIMULATED",
        "/ Search",
        "s Sort",
        "F Starred",
        "g Sectors",
        "1: 1D",
        "8: 5Y",
        "0: ALL",
        "r Refresh",
        "S Status",
        "? Help",
    ] {
        assert!(screen.contains(expected), "missing rail hint {expected:?}");
    }

    let sector_hint = state
        .hit_targets
        .iter()
        .find(|target| target.action == UiAction::BeginSectorShortcut)
        .expect("sector shortcut rail target")
        .clone();
    assert!(
        handle_event(
            &mut state,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: sector_hint.rect.x,
                row: sector_hint.rect.y,
                modifiers: KeyModifiers::NONE,
            }),
        )
        .is_empty()
    );
    assert!(state.sector_shortcut_pending);

    assert!(press(&mut state, KeyCode::Char('?'), KeyModifiers::NONE).is_empty());
    assert!(!state.sector_shortcut_pending);
    let help = screen_text(&render_at(&mut state, 80, 24));
    for expected in [
        "HELP",
        "Navigate",
        "arrows or h j k l",
        "Starred      F",
        "Ranges       1..9, 0 or [ ]",
        "Sectors      g then c s h e t f i m u",
        "Detail       Left/Right chart, Up/Down news",
        "Quit         q",
    ] {
        assert!(help.contains(expected), "missing help text {expected:?}");
    }

    assert!(press(&mut state, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    assert!(press(&mut state, KeyCode::Char('S'), KeyModifiers::SHIFT).is_empty());
    assert_eq!(state.overlay, Some(Overlay::Sync));
    let status = screen_text(&render_at(&mut state, 80, 24));
    for expected in [
        "DATA STATUS",
        "Auto refresh Disabled (demo/offline)",
        "Coverage     Refresh requests every retained ticker",
        "Stale data   Provider observation is over 72h old",
    ] {
        assert!(
            status.contains(expected),
            "missing data status text {expected:?}"
        );
    }
    let price_cache = format!(
        "Price cache {}",
        fixture_time()
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
    );
    assert!(
        status.contains(&price_cache),
        "missing data status text {price_cache:?}"
    );

    state.simulated_data = false;
    state.auto_refresh_interval = Some(std::time::Duration::from_secs(300));
    let live_status = screen_text(&render_at(&mut state, 80, 24));
    assert!(live_status.contains("Auto refresh Every 5m"));
}

#[test]
fn every_range_remains_clickable_at_the_minimum_supported_height() {
    let mut state = fixture_state();
    state.route = Route::Sector(Sector::Technology);
    render_at(&mut state, 60, 20);

    for range in DateRange::ALL {
        assert!(
            state
                .hit_targets
                .iter()
                .any(|target| target.action == UiAction::SelectRange(range)),
            "{} range has no compact rail target",
            range.label()
        );
    }
}

fn fixture_state() -> UiState {
    let mut tiles = Vec::new();
    for (sector_index, sector) in Sector::ALL.into_iter().enumerate() {
        for rank in 0..2 {
            let symbol = match (sector, rank) {
                (Sector::Technology, 0) => "ACME".to_owned(),
                (Sector::Technology, 1) => "BETA".to_owned(),
                _ => format!("S{sector_index}{rank}"),
            };
            let name = match symbol.as_str() {
                "ACME" => "Acme Systems".to_owned(),
                "BETA" => "Beta Computing".to_owned(),
                _ => format!("{} Fixture {rank}", sector.label()),
            };
            let ordinal = sector_index * 2 + rank;
            let company = fixture_company(
                &symbol,
                &name,
                sector,
                u16::try_from(rank + 1).expect("fixture rank fits u16"),
                900_000_000_000.0 - ordinal as f64 * 23_000_000_000.0,
            );
            tiles.push(MarketTile {
                company,
                price: Some(82.0 + ordinal as f64 * 3.25),
                period_return: Some((ordinal as f64 - 8.5) / 100.0),
                volume: Some(1_200_000.0 + ordinal as f64 * 75_000.0),
                starred: rank == 0 && sector_index.is_multiple_of(3),
                stale: ordinal.is_multiple_of(7),
                updated_at: Some(fixture_time()),
            });
        }
    }
    let benchmark_returns = [-0.012, 0.004, 0.021];
    let benchmarks = MarketBenchmark::ALL
        .into_iter()
        .enumerate()
        .map(|(index, benchmark)| MarketTile {
            company: benchmark.company(fixture_time()),
            price: Some(510.25 - index as f64 * 34.5),
            period_return: Some(benchmark_returns[index]),
            volume: Some(40_000_000.0 + index as f64 * 5_000_000.0),
            starred: false,
            stale: false,
            updated_at: Some(fixture_time()),
        })
        .collect();

    UiState {
        tiles,
        benchmarks,
        status: "Fixture cache ready".to_owned(),
        snapshot_checkpoint: Some(fixture_time()),
        ..UiState::default()
    }
}

fn detail_state() -> UiState {
    UiState {
        route: Route::Ticker("ACME".to_owned()),
        detail: Some(fixture_detail()),
        ..fixture_state()
    }
}

fn fixture_detail() -> TickerDetail {
    let company = fixture_company(
        "ACME",
        "Acme Systems",
        Sector::Technology,
        1,
        875_000_000_000.0,
    );
    let bars = (0..48)
        .map(|index| {
            let baseline = 96.0 + index as f64 * 0.16;
            let close = baseline + (index as f64 / 4.0).sin() * 1.8;
            Bar {
                symbol: "ACME".to_owned(),
                timeframe: "1Hour".to_owned(),
                timestamp: fixture_time() - Duration::hours(47 - index),
                open: close - 0.35,
                high: close + 0.9,
                low: close - 1.1,
                close,
                volume: 800_000.0 + index as f64 * 18_000.0,
                trade_count: Some(12_000 + u64::try_from(index).expect("fixture index fits u64")),
                vwap: Some(close - 0.08),
                source: "fixture".to_owned(),
            }
        })
        .collect();
    let news = vec![
        NewsItem {
            id: "acme-analytics".to_owned(),
            headline: "Acme expands terminal analytics coverage".to_owned(),
            source: "Fixture Wire".to_owned(),
            published_at: fixture_time() - Duration::hours(3),
            url: "https://example.invalid/acme/analytics".to_owned(),
            summary: "A concise fixture summary.".to_owned(),
            symbols: vec!["ACME".to_owned()],
        },
        NewsItem {
            id: "acme-results".to_owned(),
            headline: "Acme reports steady demand across its portfolio".to_owned(),
            source: "Test Ledger".to_owned(),
            published_at: fixture_time() - Duration::days(1),
            url: "https://example.invalid/acme/results".to_owned(),
            summary: "A second concise fixture summary.".to_owned(),
            symbols: vec!["ACME".to_owned()],
        },
    ];

    TickerDetail {
        company,
        snapshot: Some(Snapshot {
            symbol: "ACME".to_owned(),
            price: Some(103.75),
            previous_close: Some(101.2),
            open: Some(101.25),
            high: Some(104.4),
            low: Some(100.8),
            volume: Some(12_450_000.0),
            updated_at: fixture_time(),
        }),
        bars,
        news,
        starred: true,
        period_return: Some(0.0725),
        sector_return: Some(0.018),
        sector_rank: Some(1),
    }
}

fn fixture_company(
    symbol: &str,
    name: &str,
    sector: Sector,
    rank: u16,
    market_cap: f64,
) -> Company {
    Company {
        symbol: symbol.to_owned(),
        name: name.to_owned(),
        sector: Some(sector),
        raw_sector: Some(sector.label().to_owned()),
        exchange: "NASDAQ".to_owned(),
        industry: "Terminal Software".to_owned(),
        market_cap: Some(market_cap),
        shares_outstanding: Some(4_000_000_000.0),
        rank: Some(rank),
        description: format!("{name} builds market analysis software used by terminal operators."),
        in_universe: true,
        retained: false,
        updated_at: fixture_time(),
    }
}

fn fixture_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 13, 15, 30, 0)
        .single()
        .expect("fixture timestamp is valid")
}

fn render_at(state: &mut UiState, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal initializes");
    terminal
        .draw(|frame| render(frame, state))
        .expect("UI renders to the test backend");
    terminal.backend().buffer().clone()
}

fn press(state: &mut UiState, code: KeyCode, modifiers: KeyModifiers) -> Vec<AppCommand> {
    handle_event(state, Event::Key(KeyEvent::new(code, modifiers)))
}

fn screen_text(buffer: &Buffer) -> String {
    let mut text = String::new();
    for cell in buffer.content() {
        text.push_str(cell.symbol());
    }
    text
}

fn rect_has_visible_cell(buffer: &Buffer, rect: Rect) -> bool {
    (rect.y..rect.bottom())
        .any(|y| (rect.x..rect.right()).any(|x| !buffer[(x, y)].symbol().trim().is_empty()))
}

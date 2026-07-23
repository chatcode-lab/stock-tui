use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;

use crate::{
    benchmarks::MarketBenchmark,
    domain::{DateRange, Sector, SortMode},
    ui::state::{DetailTab, Overlay, Route, UiAction, UiState},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    Quit,
    ReloadTiles,
    LoadTicker(String),
    ToggleFavorite(String),
    Refresh,
    Search(String),
    OpenUrl(String),
}

pub fn handle_event(state: &mut UiState, event: Event) -> Vec<AppCommand> {
    match event {
        Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            handle_key(state, key)
        }
        Event::Mouse(mouse) => handle_mouse(state, mouse),
        Event::Paste(text) if state.overlay == Some(Overlay::Search) => {
            state.sector_shortcut_pending = false;
            state.search_query.push_str(text.trim());
            vec![AppCommand::Search(state.search_query.clone())]
        }
        Event::Paste(_) => {
            state.sector_shortcut_pending = false;
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn handle_key(state: &mut UiState, key: KeyEvent) -> Vec<AppCommand> {
    if let Some(overlay) = state.overlay.clone() {
        state.sector_shortcut_pending = false;
        return handle_overlay_key(state, overlay, key);
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        state.sector_shortcut_pending = false;
        return vec![AppCommand::Quit];
    }
    if state.sector_shortcut_pending {
        if matches!(key.code, KeyCode::Esc | KeyCode::Backspace) {
            state.sector_shortcut_pending = false;
            return Vec::new();
        }
        if key.code == KeyCode::Char('g') && key.modifiers.is_empty() {
            return Vec::new();
        }
        state.sector_shortcut_pending = false;
        if key.modifiers.is_empty()
            && let KeyCode::Char(character) = key.code
            && let Some(sector) = sector_for_character(character)
        {
            return apply_action(state, UiAction::OpenSector(sector));
        }
    }
    if let Some(sector) = sector_shortcut(key) {
        return apply_action(state, UiAction::OpenSector(sector));
    }
    match key.code {
        KeyCode::Char('g') if key.modifiers.is_empty() => {
            apply_action(state, UiAction::BeginSectorShortcut)
        }
        KeyCode::Char('p') if key.modifiers.is_empty() => {
            apply_action(state, UiAction::PreviousView)
        }
        KeyCode::Char('n') if key.modifiers.is_empty() => apply_action(state, UiAction::NextView),
        KeyCode::Char('q') if key.modifiers.is_empty() => vec![AppCommand::Quit],
        KeyCode::Esc | KeyCode::Backspace => apply_action(state, UiAction::Back),
        KeyCode::Char('/') => apply_action(state, UiAction::OpenSearch),
        KeyCode::Char('s') => apply_action(state, UiAction::OpenSort),
        KeyCode::Char('F') => apply_action(state, UiAction::OpenFavorites),
        KeyCode::Char('S') => apply_action(state, UiAction::OpenSync),
        KeyCode::Char('?') => apply_action(state, UiAction::OpenHelp),
        KeyCode::Char('r') => vec![AppCommand::Refresh],
        KeyCode::Char('f') => state
            .focused_symbol()
            .map(str::to_owned)
            .map_or_else(Vec::new, |symbol| {
                apply_action(state, UiAction::ToggleFavorite(symbol))
            }),
        KeyCode::Char('[') => {
            apply_action(state, UiAction::SelectRange(state.date_range.previous()))
        }
        KeyCode::Char(']') => apply_action(state, UiAction::SelectRange(state.date_range.next())),
        KeyCode::Char('0') => apply_action(state, UiAction::SelectRange(DateRange::All)),
        KeyCode::Char(character @ '1'..='9') => {
            let index = usize::from(character as u8 - b'1');
            apply_action(state, UiAction::SelectRange(DateRange::ALL[index]))
        }
        KeyCode::Left | KeyCode::Char('h') => move_selection(state, -1, 0),
        KeyCode::Right | KeyCode::Char('l') => move_selection(state, 1, 0),
        KeyCode::Up | KeyCode::Char('k') => move_selection(state, 0, -1),
        KeyCode::Down | KeyCode::Char('j') => move_selection(state, 0, 1),
        KeyCode::Enter => activate_selection(state),
        KeyCode::Tab if matches!(state.route, Route::Ticker(_)) => {
            state.detail_tab = match state.detail_tab {
                DetailTab::Chart => DetailTab::Statistics,
                DetailTab::Statistics => DetailTab::News,
                DetailTab::News => DetailTab::Chart,
            };
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn handle_overlay_key(state: &mut UiState, overlay: Overlay, key: KeyEvent) -> Vec<AppCommand> {
    if key.code == KeyCode::Esc {
        state.overlay = None;
        return Vec::new();
    }
    match overlay {
        Overlay::Search => match key.code {
            KeyCode::Enter => state
                .search_results
                .get(state.search_selected)
                .map(|company| company.symbol.clone())
                .map_or_else(Vec::new, |symbol| {
                    apply_action(state, UiAction::SearchResult(symbol))
                }),
            KeyCode::Up => {
                state.search_selected = state.search_selected.saturating_sub(1);
                Vec::new()
            }
            KeyCode::Down => {
                state.search_selected =
                    (state.search_selected + 1).min(state.search_results.len().saturating_sub(1));
                Vec::new()
            }
            KeyCode::Backspace => {
                state.search_query.pop();
                state.search_selected = 0;
                vec![AppCommand::Search(state.search_query.clone())]
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.search_query.clear();
                state.search_selected = 0;
                vec![AppCommand::Search(String::new())]
            }
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                state.search_query.push(character);
                state.search_selected = 0;
                vec![AppCommand::Search(state.search_query.clone())]
            }
            _ => Vec::new(),
        },
        Overlay::Sort => match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let index = SortMode::ALL
                    .iter()
                    .position(|mode| *mode == state.sort)
                    .unwrap_or(0);
                state.sort = SortMode::ALL[index.saturating_sub(1)];
                vec![AppCommand::ReloadTiles]
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let index = SortMode::ALL
                    .iter()
                    .position(|mode| *mode == state.sort)
                    .unwrap_or(0);
                state.sort = SortMode::ALL[(index + 1).min(SortMode::ALL.len() - 1)];
                vec![AppCommand::ReloadTiles]
            }
            KeyCode::Enter => {
                state.overlay = None;
                vec![AppCommand::ReloadTiles]
            }
            _ => Vec::new(),
        },
        Overlay::Help | Overlay::Sync => match key.code {
            KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?') => {
                state.overlay = None;
                Vec::new()
            }
            _ => Vec::new(),
        },
    }
}

fn handle_mouse(state: &mut UiState, mouse: MouseEvent) -> Vec<AppCommand> {
    state.sector_shortcut_pending = false;
    let position = Position::new(mouse.column, mouse.row);
    match mouse.kind {
        MouseEventKind::Moved | MouseEventKind::Drag(MouseButton::Left) => {
            state.hover_at(position);
            update_chart_hover(state, position);
            Vec::new()
        }
        MouseEventKind::Down(MouseButton::Left) => state
            .action_at(position)
            .cloned()
            .map_or_else(Vec::new, |action| apply_action(state, action)),
        MouseEventKind::ScrollUp => {
            if matches!(state.route, Route::Ticker(_)) {
                move_selection(state, -1, 0)
            } else {
                apply_action(state, UiAction::SelectRange(state.date_range.previous()))
            }
        }
        MouseEventKind::ScrollDown => {
            if matches!(state.route, Route::Ticker(_)) {
                move_selection(state, 1, 0)
            } else {
                apply_action(state, UiAction::SelectRange(state.date_range.next()))
            }
        }
        _ => Vec::new(),
    }
}

fn update_chart_hover(state: &mut UiState, position: Position) {
    let Some(area) = state.chart_rect else {
        return;
    };
    if position.x < area.x
        || position.x >= area.right()
        || position.y < area.y
        || position.y >= area.bottom()
    {
        state.detail_hover = None;
        return;
    }
    if state.chart_sample_indices.is_empty() {
        return;
    }
    let relative = usize::from(position.x.saturating_sub(area.x));
    state.detail_hover = Some(relative.min(state.chart_sample_indices.len() - 1));
}

fn move_selection(state: &mut UiState, horizontal: isize, vertical: isize) -> Vec<AppCommand> {
    match state.route {
        Route::Overview => {
            state.hovered_symbol = None;
            if let Some(selected) = state.selected_benchmark {
                if vertical < 0 {
                    state.selected_sector = 6 + selected.min(2);
                    state.selected_benchmark = None;
                } else if horizontal != 0 {
                    state.selected_benchmark = Some(offset(selected, horizontal, 2));
                }
                return Vec::new();
            }
            let row = state.selected_sector / 3;
            let column = state.selected_sector % 3;
            if row == 2 && vertical > 0 {
                state.selected_benchmark = Some(column);
                return Vec::new();
            }
            let row = offset(row, vertical, 2);
            let column = offset(column, horizontal, 2);
            state.selected_sector = row * 3 + column;
        }
        Route::Sector(_) | Route::Favorites => {
            state.hovered_symbol = None;
            let columns = state.sector_columns.max(1);
            let row = state.selected_ticker / columns;
            let column = state.selected_ticker % columns;
            let count = state.visible_tiles().len();
            let max_row = count.saturating_sub(1) / columns;
            let row = offset(row, vertical, max_row);
            let column = offset(column, horizontal, columns - 1);
            state.selected_ticker = (row * columns + column).min(count.saturating_sub(1));
        }
        Route::Ticker(_) => {
            if horizontal != 0 {
                let current = state
                    .detail_hover
                    .unwrap_or_else(|| state.chart_sample_indices.len().saturating_sub(1));
                state.detail_hover = Some(offset(
                    current,
                    horizontal,
                    state.chart_sample_indices.len().saturating_sub(1),
                ));
            }
            if vertical != 0 {
                let maximum = state
                    .detail
                    .as_ref()
                    .map_or(0, |detail| detail.news.len().saturating_sub(1));
                state.selected_news = offset(state.selected_news, vertical, maximum);
            }
        }
    }
    Vec::new()
}

fn activate_selection(state: &mut UiState) -> Vec<AppCommand> {
    match state.route.clone() {
        Route::Overview => {
            if let Some(index) = state.selected_benchmark {
                apply_action(
                    state,
                    UiAction::OpenTicker(
                        MarketBenchmark::ALL[index.min(MarketBenchmark::ALL.len() - 1)]
                            .symbol
                            .to_owned(),
                    ),
                )
            } else {
                apply_action(
                    state,
                    UiAction::OpenSector(Sector::ALL[state.selected_sector.min(8)]),
                )
            }
        }
        Route::Sector(_) | Route::Favorites => state
            .visible_tiles()
            .get(state.selected_ticker)
            .map(|tile| tile.company.symbol.clone())
            .map_or_else(Vec::new, |symbol| {
                apply_action(state, UiAction::OpenTicker(symbol))
            }),
        Route::Ticker(_) => apply_action(state, UiAction::OpenNews(state.selected_news)),
    }
}

fn apply_action(state: &mut UiState, action: UiAction) -> Vec<AppCommand> {
    if !matches!(&action, UiAction::BeginSectorShortcut) {
        state.sector_shortcut_pending = false;
    }
    match action {
        UiAction::Back => {
            if state.overlay.take().is_some() {
                return Vec::new();
            }
            let current_route = state.route.clone();
            let ticker_symbol = match &current_route {
                Route::Ticker(symbol) => Some(symbol.clone()),
                _ => None,
            };
            state.route = match current_route {
                Route::Ticker(_) => state.detail_return_route.take().unwrap_or_else(|| {
                    state
                        .detail
                        .as_ref()
                        .and_then(|detail| detail.company.sector)
                        .map_or(Route::Overview, Route::Sector)
                }),
                Route::Sector(_) | Route::Favorites => {
                    state.detail_return_route = None;
                    Route::Overview
                }
                Route::Overview => Route::Overview,
            };
            state.detail = None;
            state.hovered_symbol = None;
            if let Some(symbol) = ticker_symbol
                && matches!(state.route, Route::Sector(_) | Route::Favorites)
            {
                state.select_visible_symbol(&symbol);
            } else if matches!(state.route, Route::Overview) {
                state.selected_ticker = 0;
            }
            Vec::new()
        }
        UiAction::OpenSearch => {
            state.overlay = Some(Overlay::Search);
            state.search_selected = 0;
            vec![AppCommand::Search(state.search_query.clone())]
        }
        UiAction::Refresh => vec![AppCommand::Refresh],
        UiAction::OpenFavorites => {
            state.overlay = None;
            state.route = Route::Favorites;
            state.selected_ticker = 0;
            state.selected_benchmark = None;
            state.hovered_symbol = None;
            state.detail_return_route = None;
            Vec::new()
        }
        UiAction::OpenHelp => {
            state.overlay = Some(Overlay::Help);
            Vec::new()
        }
        UiAction::OpenSync => {
            state.overlay = Some(Overlay::Sync);
            Vec::new()
        }
        UiAction::OpenSort => {
            state.overlay = Some(Overlay::Sort);
            Vec::new()
        }
        UiAction::BeginSectorShortcut => {
            state.sector_shortcut_pending = true;
            Vec::new()
        }
        UiAction::PreviousView => switch_view(state, -1),
        UiAction::NextView => switch_view(state, 1),
        UiAction::CloseOverlay => {
            state.overlay = None;
            Vec::new()
        }
        UiAction::SelectRange(range) => {
            state.date_range = range;
            state.detail_hover = None;
            if let Route::Ticker(symbol) = &state.route {
                vec![
                    AppCommand::ReloadTiles,
                    AppCommand::LoadTicker(symbol.clone()),
                ]
            } else {
                vec![AppCommand::ReloadTiles]
            }
        }
        UiAction::SelectSort(sort) => {
            state.sort = sort;
            state.overlay = None;
            vec![AppCommand::ReloadTiles]
        }
        UiAction::OpenSector(sector) => {
            state.route = Route::Sector(sector);
            state.selected_sector = Sector::ALL
                .iter()
                .position(|item| *item == sector)
                .unwrap_or(0);
            state.selected_ticker = 0;
            state.selected_benchmark = None;
            state.hovered_symbol = None;
            state.detail_return_route = None;
            Vec::new()
        }
        UiAction::OpenTicker(symbol) | UiAction::SearchResult(symbol) => {
            if matches!(state.route, Route::Overview) {
                state.selected_benchmark = MarketBenchmark::ALL
                    .iter()
                    .position(|benchmark| benchmark.symbol == symbol);
            }
            if matches!(state.route, Route::Sector(_) | Route::Favorites) {
                state.select_visible_symbol(&symbol);
                state.detail_return_route = Some(state.route.clone());
            } else if !matches!(state.route, Route::Ticker(_)) {
                state.detail_return_route = None;
            }
            state.overlay = None;
            state.route = Route::Ticker(symbol.clone());
            state.detail = None;
            state.detail_hover = None;
            state.selected_news = 0;
            state.hovered_symbol = None;
            vec![AppCommand::LoadTicker(symbol)]
        }
        UiAction::ToggleFavorite(symbol) => vec![AppCommand::ToggleFavorite(symbol)],
        UiAction::OpenNews(index) => {
            state.selected_news = index;
            state
                .detail
                .as_ref()
                .and_then(|detail| detail.news.get(index))
                .map(|item| AppCommand::OpenUrl(item.url.clone()))
                .into_iter()
                .collect()
        }
        UiAction::SelectDetailTab(tab) => {
            state.detail_tab = tab;
            Vec::new()
        }
    }
}

fn switch_view(state: &mut UiState, direction: isize) -> Vec<AppCommand> {
    match state.route.clone() {
        Route::Sector(sector) => {
            let current = Sector::ALL
                .iter()
                .position(|candidate| *candidate == sector)
                .unwrap_or(0);
            let next = cycle_index(current, direction, Sector::ALL.len());
            state.route = Route::Sector(Sector::ALL[next]);
            state.selected_sector = next;
            state.selected_ticker = state
                .selected_ticker
                .min(state.visible_tiles().len().saturating_sub(1));
            state.selected_benchmark = None;
            state.hovered_symbol = None;
            state.detail_return_route = None;
            Vec::new()
        }
        Route::Ticker(symbol) => {
            let context = state.detail_context_route();
            let symbols = state
                .detail_navigation_symbols()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let Some(current) = symbols.iter().position(|candidate| *candidate == symbol) else {
                return Vec::new();
            };
            if symbols.len() < 2 {
                return Vec::new();
            }
            let next = cycle_index(current, direction, symbols.len());
            let next_symbol = symbols[next].clone();
            if context.is_none() && MarketBenchmark::for_symbol(&symbol).is_some() {
                state.selected_benchmark = Some(next);
            } else {
                state.detail_return_route = state.detail_return_route.clone().or(context);
                state.selected_ticker = next;
            }
            apply_action(state, UiAction::OpenTicker(next_symbol))
        }
        Route::Overview | Route::Favorites => Vec::new(),
    }
}

fn cycle_index(value: usize, direction: isize, length: usize) -> usize {
    if length == 0 {
        return 0;
    }
    if direction < 0 {
        value.checked_sub(1).unwrap_or(length - 1)
    } else {
        (value + 1) % length
    }
}

fn offset(value: usize, amount: isize, maximum: usize) -> usize {
    value.saturating_add_signed(amount).min(maximum)
}

fn sector_shortcut(key: KeyEvent) -> Option<Sector> {
    let navigation_modifiers = KeyModifiers::ALT | KeyModifiers::SUPER | KeyModifiers::META;
    if key.modifiers.contains(KeyModifiers::CONTROL)
        || !key.modifiers.intersects(navigation_modifiers)
    {
        return None;
    }
    let KeyCode::Char(character) = key.code else {
        return None;
    };
    sector_for_character(character)
}

fn sector_for_character(character: char) -> Option<Sector> {
    match character.to_ascii_lowercase() {
        'c' => Some(Sector::Consumer),
        's' => Some(Sector::Services),
        'h' => Some(Sector::Healthcare),
        'e' => Some(Sector::Energy),
        't' => Some(Sector::Technology),
        'f' => Some(Sector::Financial),
        'i' => Some(Sector::Industrial),
        'm' => Some(Sector::Materials),
        'u' => Some(Sector::Utilities),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_range_shortcuts_are_direct() {
        let mut state = UiState::default();
        let eight_commands = handle_event(
            &mut state,
            Event::Key(KeyEvent::new(KeyCode::Char('8'), KeyModifiers::NONE)),
        );
        assert_eq!(state.date_range, DateRange::FiveYears);
        assert_eq!(eight_commands, vec![AppCommand::ReloadTiles]);

        let zero_commands = handle_event(
            &mut state,
            Event::Key(KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE)),
        );
        assert_eq!(state.date_range, DateRange::All);
        assert_eq!(zero_commands, vec![AppCommand::ReloadTiles]);
    }

    #[test]
    fn search_owns_printable_keys() {
        let mut state = UiState {
            overlay: Some(Overlay::Search),
            ..UiState::default()
        };
        let commands = handle_event(
            &mut state,
            Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        );
        assert_eq!(state.search_query, "q");
        assert_eq!(commands, vec![AppCommand::Search("q".to_owned())]);
    }

    #[test]
    fn overview_arrows_move_between_sectors_and_benchmarks() {
        let mut state = UiState {
            selected_sector: 7,
            ..UiState::default()
        };

        assert!(move_selection(&mut state, 0, 1).is_empty());
        assert_eq!(state.selected_benchmark, Some(1));
        assert!(move_selection(&mut state, 1, 0).is_empty());
        assert_eq!(state.selected_benchmark, Some(2));
        assert!(move_selection(&mut state, 0, -1).is_empty());
        assert_eq!(state.selected_benchmark, None);
        assert_eq!(state.selected_sector, 8);

        assert!(move_selection(&mut state, 0, 1).is_empty());
        assert_eq!(
            activate_selection(&mut state),
            vec![AppCommand::LoadTicker("QQQ".to_owned())]
        );
        assert_eq!(state.route, Route::Ticker("QQQ".to_owned()));
    }

    #[test]
    fn modified_sector_shortcuts_preserve_ctrl_c_and_plain_keys() {
        let shortcuts = [
            ('c', Sector::Consumer),
            ('s', Sector::Services),
            ('h', Sector::Healthcare),
            ('e', Sector::Energy),
            ('t', Sector::Technology),
            ('f', Sector::Financial),
            ('i', Sector::Industrial),
            ('m', Sector::Materials),
            ('u', Sector::Utilities),
        ];
        for (index, (key, sector)) in shortcuts.into_iter().enumerate() {
            let mut state = UiState {
                selected_benchmark: Some(1),
                ..UiState::default()
            };
            let modifiers = if index.is_multiple_of(2) {
                KeyModifiers::ALT
            } else {
                KeyModifiers::META
            };

            assert!(
                handle_event(
                    &mut state,
                    Event::Key(KeyEvent::new(KeyCode::Char(key), modifiers))
                )
                .is_empty()
            );
            assert_eq!(state.route, Route::Sector(sector));
            assert_eq!(state.selected_benchmark, None);
        }

        let mut state = UiState::default();
        assert_eq!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(
                    KeyCode::Char('c'),
                    KeyModifiers::CONTROL | KeyModifiers::ALT,
                ))
            ),
            vec![AppCommand::Quit]
        );
        assert!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            )
            .is_empty()
        );
        assert_eq!(state.overlay, Some(Overlay::Sort));
    }

    #[test]
    fn terminal_safe_sector_chord_opens_every_sector() {
        let shortcuts = [
            ('c', Sector::Consumer),
            ('s', Sector::Services),
            ('h', Sector::Healthcare),
            ('e', Sector::Energy),
            ('t', Sector::Technology),
            ('f', Sector::Financial),
            ('i', Sector::Industrial),
            ('m', Sector::Materials),
            ('u', Sector::Utilities),
        ];
        for (key, sector) in shortcuts {
            let mut state = UiState {
                selected_benchmark: Some(1),
                ..UiState::default()
            };

            assert!(
                handle_event(
                    &mut state,
                    Event::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
                )
                .is_empty()
            );
            assert!(state.sector_shortcut_pending);
            assert!(
                handle_event(
                    &mut state,
                    Event::Key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE))
                )
                .is_empty()
            );
            assert_eq!(state.route, Route::Sector(sector));
            assert!(!state.sector_shortcut_pending);
            assert_eq!(state.selected_benchmark, None);
        }
    }

    #[test]
    fn sector_chord_cancels_cleanly_and_search_owns_its_text() {
        let mut state = UiState {
            route: Route::Sector(Sector::Technology),
            ..UiState::default()
        };
        assert!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            )
            .is_empty()
        );
        assert!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            )
            .is_empty()
        );
        assert_eq!(state.route, Route::Sector(Sector::Technology));
        assert!(!state.sector_shortcut_pending);

        assert!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            )
            .is_empty()
        );
        assert!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
            )
            .is_empty()
        );
        assert!(!state.sector_shortcut_pending);
        assert_eq!(state.route, Route::Sector(Sector::Technology));

        assert!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            )
            .is_empty()
        );
        assert_eq!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
            ),
            vec![AppCommand::Refresh]
        );
        assert!(!state.sector_shortcut_pending);

        assert!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            )
            .is_empty()
        );
        assert_eq!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
            ),
            vec![AppCommand::Search(String::new())]
        );
        assert_eq!(state.overlay, Some(Overlay::Search));
        assert!(!state.sector_shortcut_pending);
        assert_eq!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            ),
            vec![AppCommand::Search("g".to_owned())]
        );
        assert_eq!(state.search_query, "g");

        state.sector_shortcut_pending = true;
        state.overlay = Some(Overlay::Help);
        assert!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
            )
            .is_empty()
        );
        assert_eq!(state.route, Route::Sector(Sector::Technology));
        assert!(!state.sector_shortcut_pending);

        state.overlay = None;
        state.sector_shortcut_pending = true;
        assert!(
            handle_event(
                &mut state,
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Moved,
                    column: 0,
                    row: 0,
                    modifiers: KeyModifiers::NONE,
                })
            )
            .is_empty()
        );
        assert!(!state.sector_shortcut_pending);
    }
}

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;

use crate::{
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
            state.search_query.push_str(text.trim());
            vec![AppCommand::Search(state.search_query.clone())]
        }
        _ => Vec::new(),
    }
}

fn handle_key(state: &mut UiState, key: KeyEvent) -> Vec<AppCommand> {
    if let Some(overlay) = state.overlay.clone() {
        return handle_overlay_key(state, overlay, key);
    }
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            vec![AppCommand::Quit]
        }
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
        KeyCode::Char(character @ '1'..='7') => {
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
            let row = state.selected_sector / 3;
            let column = state.selected_sector % 3;
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
        Route::Ticker(_) => match state.detail_tab {
            DetailTab::Chart => {
                let current = state
                    .detail_hover
                    .unwrap_or_else(|| state.chart_sample_indices.len().saturating_sub(1));
                state.detail_hover = Some(offset(
                    current,
                    horizontal + vertical,
                    state.chart_sample_indices.len().saturating_sub(1),
                ));
            }
            DetailTab::News => {
                let maximum = state
                    .detail
                    .as_ref()
                    .map_or(0, |detail| detail.news.len().saturating_sub(1));
                state.selected_news = offset(state.selected_news, horizontal + vertical, maximum);
            }
            DetailTab::Statistics => {}
        },
    }
    Vec::new()
}

fn activate_selection(state: &mut UiState) -> Vec<AppCommand> {
    match state.route.clone() {
        Route::Overview => apply_action(
            state,
            UiAction::OpenSector(Sector::ALL[state.selected_sector.min(8)]),
        ),
        Route::Sector(_) | Route::Favorites => state
            .visible_tiles()
            .get(state.selected_ticker)
            .map(|tile| tile.company.symbol.clone())
            .map_or_else(Vec::new, |symbol| {
                apply_action(state, UiAction::OpenTicker(symbol))
            }),
        Route::Ticker(_) if state.detail_tab == DetailTab::News => {
            apply_action(state, UiAction::OpenNews(state.selected_news))
        }
        Route::Ticker(_) => Vec::new(),
    }
}

fn apply_action(state: &mut UiState, action: UiAction) -> Vec<AppCommand> {
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
            state.hovered_symbol = None;
            state.detail_return_route = None;
            Vec::new()
        }
        UiAction::OpenTicker(symbol) | UiAction::SearchResult(symbol) => {
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

fn offset(value: usize, amount: isize, maximum: usize) -> usize {
    value.saturating_add_signed(amount).min(maximum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_range_shortcuts_are_direct() {
        let mut state = UiState::default();
        let commands = handle_event(
            &mut state,
            Event::Key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE)),
        );
        assert_eq!(state.date_range, DateRange::FiveYears);
        assert_eq!(commands, vec![AppCommand::ReloadTiles]);
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
}

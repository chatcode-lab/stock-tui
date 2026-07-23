use std::time::Duration;

use chrono::{DateTime, Utc};
use ratatui::layout::{Position, Rect};

use crate::{
    domain::{Company, DateRange, MarketTile, Sector, SortMode, SyncProgress, TickerDetail},
    palette::Theme,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Overview,
    Sector(Sector),
    Ticker(String),
    Favorites,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Chart,
    Statistics,
    News,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    Search,
    Sort,
    Help,
    Sync,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiAction {
    Back,
    OpenSearch,
    Refresh,
    OpenFavorites,
    OpenHelp,
    OpenSync,
    OpenSort,
    CloseOverlay,
    SelectRange(DateRange),
    SelectSort(SortMode),
    OpenSector(Sector),
    OpenTicker(String),
    ToggleFavorite(String),
    SearchResult(String),
    OpenNews(usize),
    SelectDetailTab(DetailTab),
}

#[derive(Debug, Clone)]
pub struct HitTarget {
    pub rect: Rect,
    pub action: UiAction,
    pub hover_symbol: Option<String>,
}

impl HitTarget {
    #[must_use]
    pub fn contains(&self, position: Position) -> bool {
        position.x >= self.rect.x
            && position.x < self.rect.right()
            && position.y >= self.rect.y
            && position.y < self.rect.bottom()
    }
}

#[derive(Debug, Clone)]
pub struct UiState {
    pub route: Route,
    pub overlay: Option<Overlay>,
    pub date_range: DateRange,
    pub sort: SortMode,
    pub tiles: Vec<MarketTile>,
    pub detail: Option<TickerDetail>,
    pub search_query: String,
    pub search_results: Vec<Company>,
    pub search_selected: usize,
    pub selected_sector: usize,
    pub selected_ticker: usize,
    pub sector_columns: usize,
    pub detail_return_route: Option<Route>,
    pub detail_tab: DetailTab,
    pub selected_news: usize,
    pub detail_hover: Option<usize>,
    pub chart_rect: Option<Rect>,
    pub chart_sample_indices: Vec<usize>,
    pub hovered_symbol: Option<String>,
    pub hit_targets: Vec<HitTarget>,
    pub sync: SyncProgress,
    pub status: String,
    pub snapshot_checkpoint: Option<DateTime<Utc>>,
    pub auto_refresh_interval: Option<Duration>,
    pub theme: Theme,
    pub simulated_data: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            route: Route::Overview,
            overlay: None,
            date_range: DateRange::Day,
            sort: SortMode::MarketCap,
            tiles: Vec::new(),
            detail: None,
            search_query: String::new(),
            search_results: Vec::new(),
            search_selected: 0,
            selected_sector: 0,
            selected_ticker: 0,
            sector_columns: 10,
            detail_return_route: None,
            detail_tab: DetailTab::Chart,
            selected_news: 0,
            detail_hover: None,
            chart_rect: None,
            chart_sample_indices: Vec::new(),
            hovered_symbol: None,
            hit_targets: Vec::new(),
            sync: SyncProgress::default(),
            status: "Loading local cache".to_owned(),
            snapshot_checkpoint: None,
            auto_refresh_interval: None,
            theme: Theme::detect(),
            simulated_data: false,
        }
    }
}

impl UiState {
    pub fn begin_frame(&mut self) {
        self.hit_targets.clear();
        self.chart_rect = None;
        self.chart_sample_indices.clear();
    }

    pub fn register(&mut self, rect: Rect, action: UiAction, hover_symbol: Option<String>) {
        if rect.width > 0 && rect.height > 0 {
            self.hit_targets.push(HitTarget {
                rect,
                action,
                hover_symbol,
            });
        }
    }

    #[must_use]
    pub fn action_at(&self, position: Position) -> Option<&UiAction> {
        self.hit_targets
            .iter()
            .rev()
            .find(|target| target.contains(position))
            .map(|target| &target.action)
    }

    pub fn hover_at(&mut self, position: Position) {
        let target = self
            .hit_targets
            .iter()
            .rev()
            .find(|target| target.contains(position))
            .map(|target| (target.action.clone(), target.hover_symbol.clone()));

        if self.overlay.is_none() {
            match target.as_ref().map(|(action, _)| action) {
                Some(UiAction::OpenSector(sector)) if matches!(self.route, Route::Overview) => {
                    self.selected_sector = Sector::ALL
                        .iter()
                        .position(|candidate| candidate == sector)
                        .unwrap_or(self.selected_sector);
                }
                Some(UiAction::OpenTicker(symbol))
                    if matches!(self.route, Route::Sector(_) | Route::Favorites) =>
                {
                    self.select_visible_symbol(symbol);
                }
                Some(UiAction::OpenNews(index)) if matches!(self.route, Route::Ticker(_)) => {
                    self.selected_news = *index;
                }
                _ => {}
            }
        }

        self.hovered_symbol = if matches!(self.route, Route::Overview) && self.overlay.is_none() {
            None
        } else {
            target.and_then(|(_, symbol)| symbol)
        };
    }

    #[must_use]
    pub fn visible_tiles(&self) -> Vec<&MarketTile> {
        match self.route {
            Route::Sector(sector) => self
                .tiles
                .iter()
                .filter(|tile| tile.company.sector == Some(sector))
                .collect(),
            Route::Favorites => self.tiles.iter().filter(|tile| tile.starred).collect(),
            _ => self.tiles.iter().collect(),
        }
    }

    #[must_use]
    pub fn focused_symbol(&self) -> Option<&str> {
        match &self.route {
            Route::Ticker(symbol) => Some(symbol),
            Route::Overview => None,
            Route::Sector(_) | Route::Favorites => self.hovered_symbol.as_deref().or_else(|| {
                self.visible_tiles()
                    .get(self.selected_ticker)
                    .map(|tile| tile.company.symbol.as_str())
            }),
        }
    }

    pub fn select_visible_symbol(&mut self, symbol: &str) {
        if let Some(index) = self
            .visible_tiles()
            .iter()
            .position(|tile| tile.company.symbol == symbol)
        {
            self.selected_ticker = index;
        }
    }
}

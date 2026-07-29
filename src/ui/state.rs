use std::time::Duration;

use chrono::{DateTime, Utc};
use ratatui::layout::{Position, Rect};

use crate::{
    benchmarks::MarketBenchmark,
    domain::{Company, DateRange, MarketTile, Sector, SortMode, SyncProgress, TickerDetail},
    palette::Theme,
    ui::layout::SectorView,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectorMetric {
    Price,
    RelativeGain,
    AbsoluteGain,
    SectorRelativeGain,
    MarketCap,
    Volume,
}

impl SectorMetric {
    #[must_use]
    pub const fn for_sort(sort: SortMode) -> Self {
        match sort {
            SortMode::MarketCap => Self::MarketCap,
            SortMode::Gainers => Self::RelativeGain,
            SortMode::Volume => Self::Volume,
            SortMode::Alphabetical => Self::Price,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Price => "Price",
            Self::RelativeGain => "Return",
            Self::AbsoluteGain => "Price change",
            Self::SectorRelativeGain => "Vs sector",
            Self::MarketCap => "Market cap",
            Self::Volume => "Volume",
        }
    }

    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Price => Self::RelativeGain,
            Self::RelativeGain => Self::AbsoluteGain,
            Self::AbsoluteGain => Self::SectorRelativeGain,
            Self::SectorRelativeGain => Self::MarketCap,
            Self::MarketCap => Self::Volume,
            Self::Volume => Self::Price,
        }
    }
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
    BeginSectorShortcut,
    PreviousView,
    NextView,
    CloseOverlay,
    SelectRange(DateRange),
    SelectSort(SortMode),
    CycleSectorMetric,
    ToggleSortDirection,
    ToggleSectorView,
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
    pub sort_descending: bool,
    pub sector_metric: SectorMetric,
    pub sector_view: SectorView,
    pub tiles: Vec<MarketTile>,
    pub favorite_tiles: Vec<MarketTile>,
    pub benchmarks: Vec<MarketTile>,
    pub selected_benchmark: Option<usize>,
    pub sector_shortcut_pending: bool,
    pub detail: Option<TickerDetail>,
    pub search_query: String,
    pub search_results: Vec<Company>,
    pub search_selected: usize,
    pub selected_sector: usize,
    pub selected_ticker: usize,
    pub sector_columns: usize,
    pub sector_rows: usize,
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
    pub data_provider_label: String,
    pub simulated_data: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            route: Route::Overview,
            overlay: None,
            date_range: DateRange::Day,
            sort: SortMode::MarketCap,
            sort_descending: true,
            sector_metric: SectorMetric::MarketCap,
            sector_view: SectorView::Grid,
            tiles: Vec::new(),
            favorite_tiles: Vec::new(),
            benchmarks: Vec::new(),
            selected_benchmark: None,
            sector_shortcut_pending: false,
            detail: None,
            search_query: String::new(),
            search_results: Vec::new(),
            search_selected: 0,
            selected_sector: 0,
            selected_ticker: 0,
            sector_columns: 10,
            sector_rows: 10,
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
            data_provider_label: "Unconfigured".to_owned(),
            simulated_data: false,
        }
    }
}

impl UiState {
    pub fn focus_overview_sector(&mut self, index: usize) {
        self.selected_sector = index.min(Sector::ALL.len() - 1);
        self.selected_benchmark = None;
    }

    pub fn focus_overview_benchmark(&mut self, index: usize) {
        self.selected_benchmark = Some(index.min(MarketBenchmark::ALL.len() - 1));
    }

    #[must_use]
    pub fn overview_sector_is_focused(&self, index: usize) -> bool {
        self.selected_benchmark.is_none() && self.selected_sector == index
    }

    pub fn select_sort(&mut self, sort: SortMode) {
        self.sort = sort;
        self.sort_descending = sort.default_descending();
        self.sector_metric = SectorMetric::for_sort(sort);
    }

    pub fn toggle_sort_direction_in_memory(&mut self) {
        let selected_symbol = self.selected_context_symbol();
        self.sort_descending = !self.sort_descending;
        self.reverse_current_tile_order();
        self.hovered_symbol = None;
        if let Some(symbol) = selected_symbol {
            self.restore_context_selection(&symbol);
        }
    }

    /// Applies the selected direction after loading tiles in a sort mode's natural order.
    pub fn orient_default_ordered_tiles(&mut self) {
        if self.sort_descending != self.sort.default_descending() {
            self.reverse_current_tile_order();
        }
    }

    fn reverse_current_tile_order(&mut self) {
        for sector in Sector::ALL
            .into_iter()
            .map(Some)
            .chain(std::iter::once(None))
        {
            let indices = self
                .tiles
                .iter()
                .enumerate()
                .filter_map(|(index, tile)| (tile.company.sector == sector).then_some(index))
                .take(100)
                .collect::<Vec<_>>();
            for offset in 0..indices.len() / 2 {
                self.tiles
                    .swap(indices[offset], indices[indices.len() - 1 - offset]);
            }
        }
        let visible_favorites = self.favorite_tiles.len().min(100);
        self.favorite_tiles[..visible_favorites].reverse();
    }

    fn selected_context_symbol(&self) -> Option<String> {
        match &self.route {
            Route::Sector(_) | Route::Favorites => self
                .visible_tiles()
                .get(self.selected_ticker)
                .map(|tile| tile.company.symbol.clone()),
            Route::Ticker(symbol) => Some(symbol.clone()),
            Route::Overview => None,
        }
    }

    fn restore_context_selection(&mut self, symbol: &str) {
        let index = match self.route.clone() {
            Route::Sector(_) | Route::Favorites => self
                .visible_tiles()
                .iter()
                .position(|tile| tile.company.symbol == symbol),
            Route::Ticker(_) => match self.detail_context_route() {
                Some(Route::Sector(sector)) => self
                    .tiles
                    .iter()
                    .filter(|tile| tile.company.sector == Some(sector))
                    .take(100)
                    .position(|tile| tile.company.symbol == symbol),
                Some(Route::Favorites) => self
                    .favorite_tiles
                    .iter()
                    .take(100)
                    .position(|tile| tile.company.symbol == symbol),
                Some(Route::Overview) | Some(Route::Ticker(_)) | None => None,
            },
            Route::Overview => None,
        };
        if let Some(index) = index {
            self.selected_ticker = index;
        }
    }

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
                    let index = Sector::ALL
                        .iter()
                        .position(|candidate| candidate == sector)
                        .unwrap_or(self.selected_sector);
                    self.focus_overview_sector(index);
                }
                Some(UiAction::OpenTicker(symbol)) if matches!(self.route, Route::Overview) => {
                    if let Some(index) = MarketBenchmark::ALL
                        .iter()
                        .position(|benchmark| benchmark.symbol == symbol)
                    {
                        self.focus_overview_benchmark(index);
                    }
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
                .take(100)
                .collect(),
            Route::Favorites => self.favorite_tiles.iter().take(100).collect(),
            _ => self.tiles.iter().collect(),
        }
    }

    #[must_use]
    pub fn detail_context_route(&self) -> Option<Route> {
        if let Some(route @ (Route::Sector(_) | Route::Favorites)) = &self.detail_return_route {
            return Some(route.clone());
        }
        let Route::Ticker(symbol) = &self.route else {
            return None;
        };
        self.detail
            .as_ref()
            .and_then(|detail| detail.company.sector)
            .or_else(|| {
                self.tiles
                    .iter()
                    .find(|tile| tile.company.symbol == *symbol)
                    .and_then(|tile| tile.company.sector)
            })
            .map(Route::Sector)
    }

    #[must_use]
    pub fn detail_navigation_symbols(&self) -> Vec<&str> {
        let Route::Ticker(symbol) = &self.route else {
            return Vec::new();
        };
        let context = self.detail_context_route();
        if context.is_none() && MarketBenchmark::for_symbol(symbol).is_some() {
            return MarketBenchmark::ALL
                .iter()
                .map(|benchmark| benchmark.symbol)
                .collect();
        }
        match context {
            Some(Route::Sector(sector)) => self
                .tiles
                .iter()
                .filter(|tile| tile.company.sector == Some(sector))
                .take(100)
                .map(|tile| tile.company.symbol.as_str())
                .collect(),
            Some(Route::Favorites) => self
                .favorite_tiles
                .iter()
                .take(100)
                .map(|tile| tile.company.symbol.as_str())
                .collect(),
            Some(Route::Overview) | Some(Route::Ticker(_)) | None => Vec::new(),
        }
    }

    #[must_use]
    pub fn detail_rank(&self) -> Option<(usize, usize)> {
        let Route::Ticker(symbol) = &self.route else {
            return None;
        };
        let symbols = self.detail_navigation_symbols();
        symbols
            .iter()
            .position(|candidate| *candidate == symbol)
            .map(|index| (index + 1, symbols.len()))
    }

    #[must_use]
    pub fn tile(&self, symbol: &str) -> Option<&MarketTile> {
        self.tiles
            .iter()
            .chain(&self.favorite_tiles)
            .chain(&self.benchmarks)
            .find(|tile| tile.company.symbol == symbol)
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

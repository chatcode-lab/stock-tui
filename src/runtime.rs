use std::{collections::HashSet, io, time::Duration};

use anyhow::{Context, Result};
use chrono::Utc;
use crossterm::event::EventStream;
use futures_util::StreamExt;
use tokio::{sync::mpsc, time::Instant};
use tokio_util::sync::{CancellationToken, DropGuard};

use crate::{
    app::{AppCommand, handle_event},
    benchmarks,
    config::{ProviderKind, Settings},
    demo,
    domain::{Company, SyncPhase, SyncProgress},
    providers::{AlpacaProvider, ProviderSet, StockApiProvider},
    storage::Storage,
    sync::{self, SyncCommand, SyncEvent},
    terminal::{TerminalSession, copy_to_terminal_clipboard},
    ui::{self, state::Route, state::UiState},
};

enum CatalogEvent {
    Applied(String),
    Failed(String),
}

struct WorkerCancellation {
    token: CancellationToken,
    _guard: DropGuard,
}

impl WorkerCancellation {
    fn new(token: CancellationToken) -> Self {
        Self {
            _guard: token.clone().drop_guard(),
            token,
        }
    }

    fn cancel(self) {
        self.token.cancel();
    }
}

pub async fn run(settings: Settings) -> Result<()> {
    let storage = Storage::open(&settings.db_path)?;
    let mut providers = if settings.demo || settings.offline {
        None
    } else {
        Some(configured_providers(&settings)?)
    };
    let (market_context, reset_incompatible_cache) = if let Some(provider) = providers.as_ref() {
        let preparation = storage.prepare_live_cache(&provider.cache_identity())?;
        (provider.market_context().clone(), preparation.was_reset())
    } else if settings.offline {
        (storage.market_context()?.unwrap_or_default(), false)
    } else {
        (Default::default(), false)
    };
    let loaded_catalog = if settings.demo {
        None
    } else {
        Some(
            crate::universe::load_companies(
                Utc::now(),
                &settings.cache_dir,
                None,
                settings.catalog_refresh_interval,
            )
            .await?,
        )
    };
    let catalog_source = loaded_catalog
        .as_ref()
        .map_or("embedded", |catalog| catalog.source.label());
    let removed_simulated_data = if settings.demo {
        false
    } else {
        let removed = storage.purge_demo_data_for_live()?;
        bootstrap_companies(
            &storage,
            loaded_catalog
                .expect("live mode resolves a catalog")
                .companies,
            !(reset_incompatible_cache || removed),
        )?;
        removed
    };

    let mut state = UiState {
        status: if reset_incompatible_cache {
            format!(
                "Cleared cache from a different provider, feed, or market; waiting for {} sync",
                settings.provider.display_name()
            )
        } else if removed_simulated_data {
            format!(
                "Removed simulated cache data; waiting for {} sync",
                settings.provider.display_name()
            )
        } else if settings.offline {
            format!("Offline cache · {catalog_source} catalog")
        } else {
            if settings.provider == ProviderKind::Alpaca {
                format!(
                    "{} cache · {} feed · {catalog_source} catalog",
                    settings.mode_label(),
                    settings.feed
                )
            } else {
                format!("{} cache · {catalog_source} catalog", settings.mode_label())
            }
        },
        data_provider_label: if settings.demo {
            "Simulated market".to_owned()
        } else if settings.offline {
            "Offline cache".to_owned()
        } else {
            settings.provider.display_name().to_owned()
        },
        simulated_data: settings.demo,
        auto_refresh_interval: (!settings.demo && !settings.offline)
            .then_some(settings.refresh_interval),
        market_context,
        ..UiState::default()
    };
    reload_tiles(&storage, &mut state)?;
    reload_snapshot_checkpoint(&storage, &mut state)?;

    let mut terminal = TerminalSession::start().context("could not initialize terminal")?;

    let (catalog_tx, mut catalog_rx) = mpsc::unbounded_channel();
    let mut catalog_worker = (!settings.demo && !settings.offline).then(|| {
        let cache_dir = settings.cache_dir.clone();
        let remote_url = settings.catalog_url.clone();
        let refresh_after = settings.catalog_refresh_interval;
        let catalog_storage = storage.clone();
        tokio::spawn(async move {
            loop {
                let result = crate::universe::load_companies(
                    Utc::now(),
                    &cache_dir,
                    Some(&remote_url),
                    refresh_after,
                )
                .await;
                if let Ok(catalog) = result
                    && catalog.source == crate::universe::CatalogSource::Remote
                {
                    let event =
                        match install_catalog_off_thread(catalog_storage.clone(), catalog).await {
                            Ok(version) => CatalogEvent::Applied(version),
                            Err(error) => CatalogEvent::Failed(error.to_string()),
                        };
                    if catalog_tx.send(event).is_err() {
                        break;
                    }
                }
                tokio::time::sleep(refresh_after).await;
            }
        })
    });

    let (idle_tx, mut event_rx) = mpsc::unbounded_channel();
    let mut event_guard = Some(idle_tx);
    let mut sync_worker = None;
    let mut sync_cancellation = None;
    let mut demo_worker = None;
    let mut demo_cancellation = None;
    let sync_commands = if settings.demo {
        let sender = event_guard.as_ref().expect("event sender exists").clone();
        let seed_storage = storage.clone();
        let reset = settings.reset_demo;
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        demo_worker = Some(tokio::task::spawn_blocking(move || {
            if let Err(error) = seed_demo(seed_storage, reset, sender.clone(), &worker_cancellation)
            {
                let _ = sender.send(SyncEvent::Error(error.to_string()));
            }
        }));
        demo_cancellation = Some(WorkerCancellation::new(cancellation));
        None
    } else if settings.offline {
        state.sync.message = "Offline cache only".to_owned();
        None
    } else {
        event_guard.take();
        let sync::SyncHandle {
            commands,
            events,
            worker,
            cancellation,
        } = sync::spawn(
            providers.take().expect("live mode configures providers"),
            storage.clone(),
            sync::SyncOptions::new(settings.history_batch_size),
        );
        event_rx = events;
        sync_worker = Some(worker);
        sync_cancellation = Some(WorkerCancellation::new(cancellation));
        Some(commands)
    };

    let mut input = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut dirty = true;
    let mut quit = false;
    let mut last_auto_refresh = Instant::now();

    let mut result: Result<()> = async {
        while !quit {
            if dirty {
                terminal
                    .terminal_mut()
                    .draw(|frame| ui::render(frame, &mut state))?;
                dirty = false;
            }
            tokio::select! {
                input_event = input.next() => {
                    match input_event {
                        Some(Ok(event)) => {
                            let commands = handle_event(&mut state, event);
                            for command in commands {
                                let resets_auto_refresh =
                                    should_reset_auto_refresh(&command, sync_commands.is_some());
                                if execute_command(
                                    command,
                                    &storage,
                                    &mut state,
                                    sync_commands.as_ref(),
                                )? {
                                    quit = true;
                                }
                                if resets_auto_refresh {
                                    last_auto_refresh = Instant::now();
                                }
                            }
                            dirty = true;
                        }
                        Some(Err(error)) => {
                            state.status = format!("Terminal input error: {error}");
                            dirty = true;
                        }
                        None => quit = true,
                    }
                }
                Some(event) = event_rx.recv() => {
                    apply_sync_event(event, &storage, &mut state)?;
                    dirty = true;
                }
                Some(event) = catalog_rx.recv() => {
                    match event {
                        CatalogEvent::Applied(version) => finish_catalog_update(
                            &storage,
                            &mut state,
                            &version,
                            sync_commands.as_ref(),
                        )?,
                        CatalogEvent::Failed(error) => {
                            state.status = format!("SEC catalog update failed: {error}");
                        }
                    }
                    dirty = true;
                }
                _ = tick.tick() => {
                    if let Some(commands) = sync_commands.as_ref()
                        && last_auto_refresh.elapsed() >= settings.refresh_interval
                    {
                        let _ = commands.send(SyncCommand::Refresh);
                        last_auto_refresh = Instant::now();
                    }
                    dirty = state.overlay.is_some() || state.detail_hover.is_some();
                }
            }
        }
        Ok(())
    }
    .await;

    retain_first_error(
        &mut result,
        terminal
            .disable_input_modes()
            .context("could not disable terminal input modes"),
    );
    drop(input);
    if let Some(cancellation) = demo_cancellation.take() {
        cancellation.cancel();
    }
    if let Some(cancellation) = sync_cancellation.take() {
        cancellation.cancel();
    }
    if let Some(commands) = sync_commands {
        let _ = commands.send(SyncCommand::Shutdown);
    }
    drop(event_guard);
    if let Some(worker) = catalog_worker.take() {
        worker.abort();
        let _ = worker.await;
    }
    if let Some(worker) = demo_worker {
        let _ = worker.await;
    }
    if let Some(mut worker) = sync_worker
        && tokio::time::timeout(Duration::from_millis(50), &mut worker)
            .await
            .is_err()
    {
        worker.abort();
        let _ = worker.await;
    }
    retain_first_error(
        &mut result,
        terminal
            .restore()
            .context("could not restore terminal state"),
    );
    result
}

fn retain_first_error(result: &mut Result<()>, cleanup: Result<()>) {
    if result.is_ok()
        && let Err(error) = cleanup
    {
        *result = Err(error);
    }
}

#[cfg(test)]
fn bootstrap_universe(storage: &Storage) -> Result<()> {
    bootstrap_companies(
        storage,
        crate::universe::embedded_companies(Utc::now())?,
        true,
    )
}

fn bootstrap_companies(
    storage: &Storage,
    mut candidates: Vec<Company>,
    preserve_cached_state: bool,
) -> Result<()> {
    let now = Utc::now();
    storage.update_company_universe(now.date_naive(), move |existing, favorite_symbols| {
        if preserve_cached_state {
            for candidate in &mut candidates {
                if let Some(cached) = existing.get(&candidate.symbol) {
                    if candidate.shares_outstanding.is_some()
                        && same_share_estimate(candidate, cached)
                    {
                        candidate.market_cap = cached.market_cap;
                    }
                    if candidate.sector == cached.sector {
                        candidate.in_universe = cached.in_universe;
                        candidate.retained = cached.retained;
                    }
                }
            }
        }
        let candidate_symbols = candidates
            .iter()
            .map(|company| company.symbol.clone())
            .collect::<HashSet<_>>();
        candidates.extend(existing.values().filter_map(|company| {
            let replacement = crate::universe::catalog_symbol_replacement(&company.symbol)?;
            if candidate_symbols.contains(&company.symbol)
                || !candidate_symbols.contains(replacement)
            {
                return None;
            }
            let mut retired = company.clone();
            retired.sector = None;
            retired.raw_sector = None;
            retired.rank = None;
            retired.in_universe = false;
            retired.retained = favorite_symbols.contains(&retired.symbol);
            retired.updated_at = now;
            Some(retired)
        }));
        candidates.extend(benchmarks::companies(now));
        candidates
    })?;
    Ok(())
}

fn configured_providers(settings: &Settings) -> Result<ProviderSet> {
    match settings.provider {
        ProviderKind::Alpaca => Ok(AlpacaProvider::new(settings)?.into_provider_set()),
        ProviderKind::StockApi => Ok(StockApiProvider::new_authenticated(
            &settings.stock_api_url,
            settings.stock_api_news,
            settings.stock_api_token.clone(),
        )?
        .into_provider_set()),
    }
}

fn same_share_estimate(left: &Company, right: &Company) -> bool {
    match (left.shares_outstanding, right.shares_outstanding) {
        (Some(left_shares), Some(right_shares))
            if left_shares.to_bits() == right_shares.to_bits() =>
        {
            left.shares_source == right.shares_source
                && left.shares_as_of == right.shares_as_of
                && left.shares_method == right.shares_method
                && left.shares_confidence == right.shares_confidence
        }
        (None, None) => true,
        _ => false,
    }
}

fn execute_command(
    command: AppCommand,
    storage: &Storage,
    state: &mut UiState,
    sync_commands: Option<&mpsc::UnboundedSender<SyncCommand>>,
) -> Result<bool> {
    match command {
        AppCommand::Quit => return Ok(true),
        AppCommand::ReloadTiles => reload_tiles(storage, state)?,
        AppCommand::LoadTicker(symbol) => {
            load_detail(storage, state, &symbol)?;
            if let Some(commands) = sync_commands {
                let _ = commands.send(SyncCommand::LoadTicker {
                    symbol,
                    range: state.date_range,
                });
            }
        }
        AppCommand::ToggleFavorite(symbol) => {
            let starred = storage.toggle_favorite(&symbol)?;
            state.status = if starred {
                format!("{symbol} added to starred tickers")
            } else {
                format!("{symbol} removed from starred tickers")
            };
            reload_tiles(storage, state)?;
            if matches!(&state.route, Route::Ticker(current) if current == &symbol) {
                load_detail(storage, state, &symbol)?;
            }
        }
        AppCommand::Refresh => {
            if let Some(commands) = sync_commands {
                let _ = commands.send(SyncCommand::Refresh);
                state.status = "Refresh requested".to_owned();
            } else {
                state.status = "Showing locally cached data".to_owned();
            }
        }
        AppCommand::Search(query) => {
            state.search_results = storage.search(&query, 20)?;
            state.search_selected = state
                .search_selected
                .min(state.search_results.len().saturating_sub(1));
        }
        AppCommand::OpenUrl(url) => {
            if let Err(error) = webbrowser::open(&url) {
                state.status = recover_news_url(&url, &error, copy_to_terminal_clipboard);
            }
        }
    }
    Ok(false)
}

fn should_reset_auto_refresh(command: &AppCommand, remote_sync_enabled: bool) -> bool {
    remote_sync_enabled && matches!(command, AppCommand::Refresh)
}

async fn install_catalog_off_thread(
    storage: Storage,
    catalog: crate::universe::LoadedCatalog,
) -> Result<String> {
    tokio::task::spawn_blocking(move || install_catalog(&storage, catalog))
        .await
        .context("SEC catalog installer task failed")?
}

fn install_catalog(storage: &Storage, catalog: crate::universe::LoadedCatalog) -> Result<String> {
    let version = catalog
        .version
        .as_deref()
        .unwrap_or("unversioned")
        .to_owned();
    bootstrap_companies(storage, catalog.companies, true)?;
    Ok(version)
}

fn finish_catalog_update(
    storage: &Storage,
    state: &mut UiState,
    version: &str,
    sync_commands: Option<&mpsc::UnboundedSender<SyncCommand>>,
) -> Result<()> {
    reload_tiles(storage, state)?;
    if let Route::Ticker(symbol) = state.route.clone() {
        load_detail(storage, state, &symbol)?;
    }
    state.status = format!("SEC catalog updated · {version}");
    if let Some(commands) = sync_commands {
        let _ = commands.send(SyncCommand::ReconcileUniverse);
    }
    Ok(())
}

fn recover_news_url(
    url: &str,
    browser_error: &impl std::fmt::Display,
    copy: impl FnOnce(&str) -> io::Result<()>,
) -> String {
    match copy(url) {
        Ok(()) => "Browser unavailable; news URL copied to clipboard".to_owned(),
        Err(clipboard_error) => {
            format!("Could not open news URL: {browser_error}; clipboard: {clipboard_error}")
        }
    }
}

fn apply_sync_event(event: SyncEvent, storage: &Storage, state: &mut UiState) -> Result<()> {
    match event {
        SyncEvent::Progress(progress) => {
            state.status.clone_from(&progress.message);
            state.sync = progress;
        }
        SyncEvent::DataChanged => {
            reload_tiles(storage, state)?;
            if let Route::Ticker(symbol) = state.route.clone() {
                load_detail(storage, state, &symbol)?;
            }
            reload_snapshot_checkpoint(storage, state)?;
        }
        SyncEvent::TickerChanged(symbol) => {
            if matches!(&state.route, Route::Ticker(current) if current == &symbol) {
                load_detail(storage, state, &symbol)?;
            }
        }
        SyncEvent::Error(error) => {
            state.status = error.clone();
            state.sync.phase = SyncPhase::Error;
            state.sync.last_error = Some(error);
            state.sync.updated_at = Utc::now();
        }
    }
    Ok(())
}

fn reload_tiles(storage: &Storage, state: &mut UiState) -> Result<()> {
    let selected_symbol = if matches!(state.route, Route::Sector(_) | Route::Favorites) {
        state
            .visible_tiles()
            .get(state.selected_ticker)
            .map(|tile| tile.company.symbol.clone())
    } else {
        None
    };
    let now = Utc::now();
    state.tiles = storage.heatmap_tiles(state.date_range, state.sort, None, false, now)?;
    state.favorite_tiles = storage.favorite_tiles(state.date_range, state.sort, now)?;
    state.orient_default_ordered_tiles();
    state.benchmarks = storage.benchmark_tiles(state.date_range, now)?;
    state.hovered_symbol = None;
    if let Some(symbol) = selected_symbol {
        state.select_visible_symbol(&symbol);
    }
    state.selected_ticker = state
        .selected_ticker
        .min(state.visible_tiles().len().saturating_sub(1));
    Ok(())
}

fn reload_snapshot_checkpoint(storage: &Storage, state: &mut UiState) -> Result<()> {
    let scope = if state.simulated_data {
        demo::CHECKPOINT_SCOPE
    } else {
        "snapshots"
    };
    state.snapshot_checkpoint = storage.sync_checkpoint(scope)?;
    Ok(())
}

fn load_detail(storage: &Storage, state: &mut UiState, symbol: &str) -> Result<()> {
    state.detail = storage.ticker_detail(symbol, state.date_range, Utc::now(), 20)?;
    if state.detail.is_none() {
        state.status = format!("No cached data for {symbol}");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DemoCacheState {
    Empty,
    Current,
    Legacy,
}

fn classify_demo_cache(checkpoints: &HashSet<String>) -> DemoCacheState {
    if checkpoints.contains(demo::CHECKPOINT_SCOPE) {
        DemoCacheState::Current
    } else if checkpoints.is_empty() {
        DemoCacheState::Empty
    } else {
        DemoCacheState::Legacy
    }
}

fn seed_demo(
    storage: Storage,
    reset: bool,
    events: mpsc::UnboundedSender<SyncEvent>,
    cancellation: &CancellationToken,
) -> Result<()> {
    if cancellation.is_cancelled() {
        return Ok(());
    }
    let counts = storage.counts()?;
    let demo_checkpoints = storage.sync_checkpoint_scopes("demo")?;
    let cache_state = classify_demo_cache(&demo_checkpoints);
    let current_demo = cache_state == DemoCacheState::Current;
    let migrate_legacy_cache = !reset && cache_state == DemoCacheState::Legacy;
    let preserved_favorites = if migrate_legacy_cache {
        storage.favorite_symbols()?
    } else {
        Vec::new()
    };
    if cancellation.is_cancelled() {
        return Ok(());
    }
    if reset {
        storage.reset_demo_data()?;
    } else if current_demo
        && counts.companies >= demo::TOTAL_COMPANIES
        && counts.snapshots >= demo::TOTAL_COMPANIES
        && counts.bars > 0
    {
        let _ = events.send(SyncEvent::DataChanged);
        return Ok(());
    } else if migrate_legacy_cache {
        storage.reset_demo_data()?;
    }
    let _ = events.send(SyncEvent::Progress(SyncProgress {
        phase: SyncPhase::History,
        completed: 0,
        total: demo::TOTAL_COMPANIES,
        message: if migrate_legacy_cache {
            "Upgrading simulated demo identities".to_owned()
        } else {
            "Building deterministic simulated market".to_owned()
        },
        last_error: None,
        updated_at: Utc::now(),
    }));
    let now = Utc::now();
    let Some(dataset) = demo::generate_until_cancelled(now, || cancellation.is_cancelled()) else {
        return Ok(());
    };
    if cancellation.is_cancelled() {
        return Ok(());
    }
    storage.replace_universe(now.date_naive(), &dataset.companies)?;
    if migrate_legacy_cache {
        let current_symbols = dataset
            .companies
            .iter()
            .map(|company| company.symbol.as_str())
            .collect::<HashSet<_>>();
        for symbol in preserved_favorites {
            if current_symbols.contains(symbol.as_str()) {
                storage.set_favorite(&symbol, true)?;
            }
        }
    }
    if cancellation.is_cancelled() {
        return Ok(());
    }
    storage.upsert_snapshots(&dataset.snapshots)?;
    if cancellation.is_cancelled() {
        return Ok(());
    }
    if storage
        .upsert_bars_until_cancelled(&dataset.bars, || cancellation.is_cancelled())?
        .is_none()
    {
        return Ok(());
    }
    if cancellation.is_cancelled() {
        return Ok(());
    }
    storage.upsert_news(&dataset.news)?;
    if cancellation.is_cancelled() {
        return Ok(());
    }
    storage.set_sync_checkpoint(demo::CHECKPOINT_SCOPE, now)?;
    let _ = events.send(SyncEvent::Progress(SyncProgress {
        phase: SyncPhase::Complete,
        completed: demo::TOTAL_COMPANIES,
        total: demo::TOTAL_COMPANIES,
        message: "SIMULATED offline market ready".to_owned(),
        last_error: None,
        updated_at: Utc::now(),
    }));
    let _ = events.send(SyncEvent::DataChanged);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        collections::HashSet,
        io,
        sync::mpsc as std_mpsc,
        thread,
        time::{Duration as StdDuration, Instant as StdInstant},
    };

    use chrono::Utc;
    use tempfile::tempdir;

    use super::{
        DemoCacheState, bootstrap_companies, bootstrap_universe, classify_demo_cache,
        finish_catalog_update, install_catalog, install_catalog_off_thread, load_detail,
        recover_news_url, reload_tiles, should_reset_auto_refresh,
    };
    use crate::{
        app::AppCommand,
        benchmarks::MarketBenchmark,
        demo,
        domain::{Sector, Snapshot, SortMode},
        storage::Storage,
        ui::state::{Route, UiState},
        universe::{CatalogSource, LoadedCatalog},
    };

    #[test]
    fn only_remote_manual_refresh_resets_the_automatic_cadence() {
        assert!(should_reset_auto_refresh(&AppCommand::Refresh, true));
        assert!(!should_reset_auto_refresh(&AppCommand::Refresh, false));
        assert!(!should_reset_auto_refresh(&AppCommand::ReloadTiles, true));
    }

    #[test]
    fn browser_failure_copies_the_original_news_url() {
        let copied = RefCell::new(String::new());
        let status = recover_news_url("https://example.test/article", &"no browser", |value| {
            copied.replace(value.to_owned());
            Ok(())
        });

        assert_eq!(*copied.borrow(), "https://example.test/article");
        assert_eq!(status, "Browser unavailable; news URL copied to clipboard");
    }

    #[test]
    fn browser_and_clipboard_failures_are_both_reported() {
        let status = recover_news_url("https://example.test/article", &"no browser", |_| {
            Err(io::Error::other("terminal rejected OSC 52"))
        });

        assert_eq!(
            status,
            "Could not open news URL: no browser; clipboard: terminal rejected OSC 52"
        );
    }

    #[test]
    fn every_obsolete_demo_checkpoint_requires_migration() {
        assert_eq!(classify_demo_cache(&HashSet::new()), DemoCacheState::Empty);
        for scope in [
            "demo",
            "demo:sec-identities-v2",
            "demo:sec-identities-v3",
            "demo:sec-identities-v4",
        ] {
            assert_eq!(
                classify_demo_cache(&HashSet::from([scope.to_owned()])),
                DemoCacheState::Legacy
            );
        }
        assert_eq!(
            classify_demo_cache(&HashSet::from([demo::CHECKPOINT_SCOPE.to_owned()])),
            DemoCacheState::Current
        );
    }

    #[test]
    fn tile_reload_preserves_selected_symbol_and_clears_stale_hover() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = Utc::now();
        let companies = crate::universe::embedded_companies(now)?
            .into_iter()
            .filter(|company| company.sector == Some(Sector::Technology))
            .take(2)
            .collect::<Vec<_>>();
        storage.replace_memberships(now.date_naive(), Sector::Technology, &companies)?;
        storage.upsert_snapshots(
            &companies
                .iter()
                .enumerate()
                .map(|(index, company)| Snapshot {
                    symbol: company.symbol.clone(),
                    price: Some(100.0 + index as f64),
                    market_cap: None,
                    previous_close: Some(99.0),
                    open: Some(99.0),
                    high: Some(102.0),
                    low: Some(98.0),
                    volume: Some(1_000.0),
                    updated_at: now,
                })
                .collect::<Vec<_>>(),
        )?;
        let mut state = UiState {
            route: Route::Sector(Sector::Technology),
            sort: SortMode::Alphabetical,
            sort_descending: true,
            ..UiState::default()
        };
        reload_tiles(&storage, &mut state)?;
        assert_eq!(state.visible_tiles().len(), 2);
        let selected_symbol = state.visible_tiles()[0].company.symbol.clone();
        let hovered_symbol = state.visible_tiles()[1].company.symbol.clone();
        state.hovered_symbol = Some(hovered_symbol);

        state.sort_descending = false;
        reload_tiles(&storage, &mut state)?;

        assert_eq!(state.hovered_symbol, None);
        assert_eq!(
            state.visible_tiles()[state.selected_ticker].company.symbol,
            selected_symbol
        );
        assert_eq!(state.selected_ticker, 1);
        Ok(())
    }

    #[test]
    fn bootstrap_preserves_inactive_candidates_and_their_favorites() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = Utc::now();
        let mut inactive = crate::universe::embedded_companies(now)?
            .into_iter()
            .find(|company| company.rank == Some(1))
            .expect("catalog has a first-ranked company");
        let sector = inactive.sector.expect("catalog company has a sector");
        inactive.in_universe = true;
        inactive.retained = false;
        storage.upsert_companies(&[inactive.clone()])?;
        storage.set_favorite(&inactive.symbol, true)?;

        bootstrap_universe(&storage)?;

        let stored = storage
            .company(&inactive.symbol)?
            .expect("inactive catalog row remains stored");
        assert!(!stored.retained);
        assert!(!stored.in_universe);
        assert!(storage.is_favorite(&inactive.symbol)?);
        assert!(
            storage
                .memberships(sector, Some(now.date_naive()))?
                .iter()
                .all(|company| company.symbol != inactive.symbol)
        );
        for benchmark in MarketBenchmark::ALL {
            let stored = storage
                .company(benchmark.symbol)?
                .expect("benchmark proxy is bootstrapped");
            assert_eq!(stored.sector, None);
            assert!(stored.retained);
            assert!(stored.in_universe);
        }
        Ok(())
    }

    #[test]
    fn bootstrap_retires_googl_when_catalog_selects_goog() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let candidates = crate::universe::embedded_companies(Utc::now())?;
        let goog = candidates
            .iter()
            .find(|company| company.symbol == "GOOG")
            .expect("embedded catalog selects GOOG");
        let sector = goog.sector.expect("Alphabet has a sector");
        let mut stale = goog.clone();
        stale.symbol = "GOOGL".to_owned();
        stale.name = "Alphabet Inc. Class A Common Stock".to_owned();
        stale.retained = true;
        stale.in_universe = true;
        let mut discovered_goog = goog.clone();
        discovered_goog.sector = None;
        discovered_goog.raw_sector = None;
        discovered_goog.rank = None;
        discovered_goog.retained = false;
        discovered_goog.in_universe = false;
        storage.upsert_companies(&[stale, discovered_goog])?;
        storage.set_favorite("GOOGL", true)?;

        bootstrap_companies(&storage, candidates, true)?;

        let members = storage.memberships(sector, None)?;
        assert!(members.iter().any(|company| company.symbol == "GOOG"));
        assert!(members.iter().all(|company| company.symbol != "GOOGL"));
        let retired = storage.company("GOOGL")?.expect("class remains searchable");
        assert_eq!(retired.sector, None);
        assert_eq!(retired.rank, None);
        assert!(!retired.in_universe);
        assert!(retired.retained);
        assert!(storage.is_favorite("GOOGL")?);
        Ok(())
    }

    #[test]
    fn bootstrap_discards_caps_derived_from_changed_share_estimates() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = Utc::now();
        let catalog_company = crate::universe::embedded_companies(now)?
            .into_iter()
            .find(|company| company.shares_outstanding.is_some())
            .expect("catalog contains a company with a share estimate");
        let mut stale = catalog_company.clone();
        stale.market_cap = Some(3_000_000_000_000.0);
        stale.shares_method = Some("superseded_method".to_owned());
        storage.upsert_companies(&[stale])?;

        bootstrap_universe(&storage)?;

        let stored = storage
            .company(&catalog_company.symbol)?
            .expect("catalog company remains stored");
        assert_eq!(stored.market_cap, None);
        assert_eq!(stored.shares_method, catalog_company.shares_method);
        Ok(())
    }

    #[test]
    fn bootstrap_discards_removed_share_estimates_and_their_caps() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = Utc::now();
        let mut catalog_company = crate::universe::embedded_companies(now)?
            .into_iter()
            .next()
            .expect("catalog contains a company");
        catalog_company.shares_outstanding = None;
        catalog_company.shares_source = None;
        catalog_company.shares_as_of = None;
        catalog_company.shares_method = None;
        catalog_company.shares_confidence = None;
        let mut stale = catalog_company.clone();
        stale.market_cap = Some(500_000_000_000.0);
        stale.shares_outstanding = Some(50_000_000.0);
        stale.shares_source = Some("superseded_source".to_owned());
        stale.shares_as_of = Some(now.date_naive());
        stale.shares_method = Some("superseded_method".to_owned());
        stale.shares_confidence = Some("low".to_owned());
        storage.upsert_companies(&[stale])?;

        bootstrap_companies(&storage, vec![catalog_company.clone()], true)?;

        let stored = storage
            .company(&catalog_company.symbol)?
            .expect("catalog company remains stored");
        assert_eq!(stored.market_cap, None);
        assert_eq!(stored.shares_outstanding, None);
        assert_eq!(stored.shares_source, None);
        assert_eq!(stored.shares_method, None);
        Ok(())
    }

    #[test]
    fn bootstrap_replaces_legacy_description_and_preserves_cached_state() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let mut catalog_company = crate::universe::embedded_companies(Utc::now())?
            .into_iter()
            .next()
            .expect("catalog contains a company");
        catalog_company.description = "Current issuer profile.".to_owned();
        catalog_company.industry = "Current industry".to_owned();
        let mut legacy = catalog_company.clone();
        legacy.description =
            "Legacy issuer is listed on TEST and classified by the SEC.".to_owned();
        legacy.industry = "Legacy industry".to_owned();
        legacy.market_cap = Some(42_000_000.0);
        legacy.in_universe = true;
        legacy.retained = true;
        storage.upsert_companies(&[legacy])?;

        bootstrap_companies(&storage, vec![catalog_company.clone()], true)?;

        let stored = storage
            .company(&catalog_company.symbol)?
            .expect("catalog company remains stored");
        assert_eq!(stored.description, "Current issuer profile.");
        assert_eq!(stored.industry, "Current industry");
        assert_eq!(stored.market_cap, Some(42_000_000.0));
        assert!(stored.in_universe);
        assert!(stored.retained);
        Ok(())
    }

    #[test]
    fn catalog_update_refreshes_an_open_ticker_before_provider_reconciliation() -> anyhow::Result<()>
    {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let mut catalog_company = crate::universe::embedded_companies(Utc::now())?
            .into_iter()
            .next()
            .expect("catalog contains a company");
        let symbol = catalog_company.symbol.clone();
        let legacy_description =
            "Legacy issuer is listed on TEST and classified by the SEC.".to_owned();
        catalog_company.description = "Current issuer profile.".to_owned();
        let mut legacy = catalog_company.clone();
        legacy.description = legacy_description.clone();
        legacy.in_universe = true;
        legacy.retained = true;
        storage.upsert_companies(&[legacy])?;
        let mut state = UiState {
            route: Route::Ticker(symbol.clone()),
            ..UiState::default()
        };
        load_detail(&storage, &mut state, &symbol)?;
        assert_eq!(
            state
                .detail
                .as_ref()
                .expect("legacy detail is loaded")
                .company
                .description,
            legacy_description
        );
        let (commands, mut received) = tokio::sync::mpsc::unbounded_channel();

        let version = install_catalog(
            &storage,
            LoadedCatalog {
                companies: vec![catalog_company],
                source: CatalogSource::Remote,
                version: Some("profile-test".to_owned()),
            },
        )?;
        finish_catalog_update(&storage, &mut state, &version, Some(&commands))?;

        assert_eq!(
            state
                .detail
                .as_ref()
                .expect("open detail is refreshed")
                .company
                .description,
            "Current issuer profile."
        );
        assert_eq!(state.status, "SEC catalog updated · profile-test");
        assert!(matches!(
            received.try_recv(),
            Ok(crate::sync::SyncCommand::ReconcileUniverse)
        ));
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn catalog_install_does_not_block_the_ui_runtime_while_sqlite_is_busy()
    -> anyhow::Result<()> {
        let directory = tempdir()?;
        let database_path = directory.path().join("market.sqlite3");
        let storage = Storage::open(&database_path)?;
        let (ready_tx, ready_rx) = std_mpsc::channel();
        let (release_tx, release_rx) = std_mpsc::channel();
        let lock_worker = thread::spawn(move || {
            let connection =
                rusqlite::Connection::open(database_path).expect("open lock connection");
            connection
                .execute_batch("BEGIN IMMEDIATE")
                .expect("hold SQLite writer lock");
            ready_tx.send(()).expect("signal held writer lock");
            let _ = release_rx.recv_timeout(StdDuration::from_secs(2));
            connection
                .execute_batch("ROLLBACK")
                .expect("release SQLite writer lock");
        });
        ready_rx.recv_timeout(StdDuration::from_secs(2))?;

        let mut catalog_company = crate::universe::embedded_companies(Utc::now())?
            .into_iter()
            .next()
            .expect("catalog contains a company");
        catalog_company.description = "Background catalog profile.".to_owned();
        let install = tokio::spawn(install_catalog_off_thread(
            storage.clone(),
            LoadedCatalog {
                companies: vec![catalog_company],
                source: CatalogSource::Remote,
                version: Some("background-test".to_owned()),
            },
        ));
        tokio::task::yield_now().await;

        let started = StdInstant::now();
        tokio::time::sleep(StdDuration::from_millis(20)).await;
        let responsive_elapsed = started.elapsed();
        assert!(
            !install.is_finished(),
            "catalog install should still be waiting for the held writer lock"
        );
        let _ = release_tx.send(());
        let version = install.await??;
        lock_worker.join().expect("lock worker exits");

        assert!(
            responsive_elapsed < StdDuration::from_millis(500),
            "UI runtime was blocked for {responsive_elapsed:?}"
        );
        assert_eq!(version, "background-test");
        Ok(())
    }
}

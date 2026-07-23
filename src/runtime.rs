use std::{
    collections::{HashMap, HashSet},
    io::{self, Write},
    time::Duration,
};

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Utc;
use crossterm::event::EventStream;
use futures_util::StreamExt;
use tokio::{sync::mpsc, time::Instant};

use crate::{
    app::{AppCommand, handle_event},
    config::Settings,
    demo,
    domain::{Company, Sector, SyncPhase, SyncProgress},
    storage::Storage,
    sync::{self, SyncCommand, SyncEvent},
    terminal::TerminalSession,
    ui::{self, state::Route, state::UiState},
};

pub async fn run(settings: Settings) -> Result<()> {
    let storage = Storage::open(&settings.db_path)?;
    if !settings.demo {
        bootstrap_universe(&storage)?;
    }

    let mut state = UiState {
        status: format!("{} cache · {} feed", settings.mode_label(), settings.feed),
        simulated_data: settings.demo,
        ..UiState::default()
    };
    reload_tiles(&storage, &mut state)?;

    let (idle_tx, mut event_rx) = mpsc::unbounded_channel();
    let mut event_guard = Some(idle_tx);
    let mut sync_worker = None;
    let sync_commands = if settings.demo {
        let sender = event_guard.as_ref().expect("event sender exists").clone();
        let seed_storage = storage.clone();
        let reset = settings.reset_demo;
        let _seed_task = tokio::task::spawn_blocking(move || {
            if let Err(error) = seed_demo(seed_storage, reset, sender.clone()) {
                let _ = sender.send(SyncEvent::Error(error.to_string()));
            }
        });
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
        } = sync::spawn(settings.clone(), storage.clone());
        event_rx = events;
        sync_worker = Some(worker);
        Some(commands)
    };

    let mut terminal = TerminalSession::start().context("could not initialize terminal")?;
    let mut input = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut dirty = true;
    let mut quit = false;
    let mut last_auto_refresh = Instant::now();

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
                            if execute_command(
                                command,
                                &storage,
                                &mut state,
                                sync_commands.as_ref(),
                            )? {
                                quit = true;
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
    if let Some(commands) = sync_commands {
        let _ = commands.send(SyncCommand::Shutdown);
    }
    drop(event_guard);
    drop(terminal);
    if let Some(mut worker) = sync_worker
        && tokio::time::timeout(Duration::from_secs(5), &mut worker)
            .await
            .is_err()
    {
        worker.abort();
        let _ = worker.await;
    }
    Ok(())
}

fn bootstrap_universe(storage: &Storage) -> Result<()> {
    let now = Utc::now();
    let existing: HashMap<String, Company> = storage
        .companies(None, false)?
        .into_iter()
        .map(|company| (company.symbol.clone(), company))
        .collect();
    let mut candidates = crate::universe::embedded_companies(now)?;
    for candidate in &mut candidates {
        if let Some(cached) = existing.get(&candidate.symbol) {
            candidate.market_cap = cached.market_cap;
            if candidate.shares_outstanding.is_none() {
                candidate.shares_outstanding = cached.shares_outstanding;
            }
            candidate.in_universe = cached.in_universe;
        }
    }
    storage.upsert_companies(&candidates)?;
    for sector in Sector::ALL {
        let sector_candidates = candidates
            .iter()
            .filter(|company| company.sector == Some(sector))
            .cloned()
            .collect::<Vec<_>>();
        storage.replace_memberships(now.date_naive(), sector, &sector_candidates)?;
    }
    Ok(())
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

fn copy_to_terminal_clipboard(value: &str) -> io::Result<()> {
    let mut output = io::stdout().lock();
    output.write_all(terminal_clipboard_sequence(value).as_bytes())?;
    output.flush()
}

fn terminal_clipboard_sequence(value: &str) -> String {
    format!("\x1b]52;c;{}\x1b\\", STANDARD.encode(value))
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
            state.last_refresh = Some(Utc::now());
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
    let now = Utc::now();
    let mut tiles = storage.heatmap_tiles(state.date_range, state.sort, None, false, now)?;
    let existing: HashSet<String> = tiles
        .iter()
        .map(|tile| tile.company.symbol.clone())
        .collect();
    tiles.extend(
        storage
            .favorite_tiles(state.date_range, state.sort, now)?
            .into_iter()
            .filter(|tile| !existing.contains(&tile.company.symbol)),
    );
    state.tiles = tiles;
    Ok(())
}

fn load_detail(storage: &Storage, state: &mut UiState, symbol: &str) -> Result<()> {
    state.detail = storage.ticker_detail(symbol, state.date_range, Utc::now(), 20)?;
    if state.detail.is_none() {
        state.status = format!("No cached data for {symbol}");
    }
    Ok(())
}

fn seed_demo(
    storage: Storage,
    reset: bool,
    events: mpsc::UnboundedSender<SyncEvent>,
) -> Result<()> {
    let counts = storage.counts()?;
    let current_demo = storage.sync_checkpoint(demo::CHECKPOINT_SCOPE)?.is_some();
    let legacy_demo = !current_demo && storage.sync_checkpoint("demo")?.is_some();
    let migrate_legacy_cache =
        !reset && legacy_demo && counts.companies == 900 && counts.snapshots == 900;
    let preserved_favorites = if migrate_legacy_cache {
        storage.favorite_symbols()?
    } else {
        Vec::new()
    };
    if reset {
        storage.reset_demo_data()?;
    } else if current_demo && counts.companies >= 900 && counts.snapshots >= 900 && counts.bars > 0
    {
        let _ = events.send(SyncEvent::DataChanged);
        return Ok(());
    } else if migrate_legacy_cache {
        storage.reset_demo_data()?;
    }
    let _ = events.send(SyncEvent::Progress(SyncProgress {
        phase: SyncPhase::History,
        completed: 0,
        total: 900,
        message: if migrate_legacy_cache {
            "Upgrading simulated demo identities".to_owned()
        } else {
            "Building deterministic simulated market".to_owned()
        },
        last_error: None,
        updated_at: Utc::now(),
    }));
    let now = Utc::now();
    let dataset = demo::generate(now);
    storage.replace_universe(now.date_naive(), &dataset.companies)?;
    storage.upsert_snapshots(&dataset.snapshots)?;
    storage.upsert_bars(&dataset.bars)?;
    storage.upsert_news(&dataset.news)?;
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
    storage.set_sync_checkpoint(demo::CHECKPOINT_SCOPE, now)?;
    let _ = events.send(SyncEvent::Progress(SyncProgress {
        phase: SyncPhase::Complete,
        completed: 900,
        total: 900,
        message: "SIMULATED offline market ready".to_owned(),
        last_error: None,
        updated_at: Utc::now(),
    }));
    let _ = events.send(SyncEvent::DataChanged);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, io};

    use super::{recover_news_url, terminal_clipboard_sequence};

    #[test]
    fn terminal_clipboard_uses_osc_52_with_a_base64_payload() {
        assert_eq!(
            terminal_clipboard_sequence("https://example.test/news?a=1&b=2"),
            "\u{1b}]52;c;aHR0cHM6Ly9leGFtcGxlLnRlc3QvbmV3cz9hPTEmYj0y\u{1b}\\"
        );
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
}

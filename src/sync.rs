use std::{collections::HashMap, sync::Arc};

use chrono::{Duration as ChronoDuration, Utc};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{
    config::Settings,
    domain::{Company, DateRange, Sector, SyncPhase, SyncProgress},
    providers::{AlpacaProvider, MarketDataProvider, NewsProvider},
    storage::Storage,
};

#[derive(Debug, Clone)]
pub enum SyncCommand {
    Refresh,
    LoadTicker { symbol: String, range: DateRange },
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum SyncEvent {
    Progress(SyncProgress),
    DataChanged,
    TickerChanged(String),
    Error(String),
}

pub struct SyncHandle {
    pub commands: mpsc::UnboundedSender<SyncCommand>,
    pub events: mpsc::UnboundedReceiver<SyncEvent>,
    pub worker: JoinHandle<()>,
}

pub fn spawn(settings: Settings, storage: Storage) -> SyncHandle {
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let worker = tokio::spawn(run_worker(settings, storage, command_rx, event_tx));
    SyncHandle {
        commands: command_tx,
        events: event_rx,
        worker,
    }
}

async fn run_worker(
    settings: Settings,
    storage: Storage,
    mut commands: mpsc::UnboundedReceiver<SyncCommand>,
    events: mpsc::UnboundedSender<SyncEvent>,
) {
    let provider = match AlpacaProvider::new(&settings) {
        Ok(provider) => Arc::new(provider),
        Err(error) => {
            tracing::warn!(error = %error, "Alpaca provider initialization failed");
            let _ = events.send(SyncEvent::Error(error.to_string()));
            return;
        }
    };
    let cancellation = CancellationToken::new();
    let snapshots_ready = match refresh_snapshots(&storage, provider.as_ref(), &events).await {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(error = %error, "initial snapshot refresh failed");
            let _ = events.send(SyncEvent::Error(error));
            false
        }
    };
    let mut history_task = snapshots_ready.then(|| {
        tokio::spawn(backfill_history(
            storage.clone(),
            Arc::clone(&provider),
            events.clone(),
            cancellation.child_token(),
            settings.history_batch_size,
        ))
    });
    let assets_task = tokio::spawn(refresh_assets(
        storage.clone(),
        Arc::clone(&provider),
        events.clone(),
    ));

    while let Some(command) = commands.recv().await {
        match command {
            SyncCommand::Refresh => {
                match refresh_snapshots(&storage, provider.as_ref(), &events).await {
                    Ok(()) if history_task.as_ref().is_none_or(JoinHandle::is_finished) => {
                        if let Some(task) = history_task.take() {
                            let _ = task.await;
                        }
                        history_task = Some(tokio::spawn(backfill_history(
                            storage.clone(),
                            Arc::clone(&provider),
                            events.clone(),
                            cancellation.child_token(),
                            settings.history_batch_size,
                        )));
                    }
                    Ok(()) => {}
                    Err(error) => {
                        tracing::warn!(error = %error, "snapshot refresh failed");
                        let _ = events.send(SyncEvent::Error(error));
                    }
                }
            }
            SyncCommand::LoadTicker { symbol, range } => {
                if let Err(error) =
                    refresh_ticker(&storage, provider.as_ref(), &events, &symbol, range).await
                {
                    tracing::warn!(%symbol, error = %error, "ticker refresh failed");
                    let _ = events.send(SyncEvent::Error(error));
                }
            }
            SyncCommand::Shutdown => break,
        }
    }
    cancellation.cancel();
    if let Some(task) = history_task {
        let _ = task.await;
    }
    let _ = assets_task.await;
}

async fn refresh_snapshots(
    storage: &Storage,
    provider: &AlpacaProvider,
    events: &mpsc::UnboundedSender<SyncEvent>,
) -> Result<(), String> {
    let companies = storage
        .companies(None, false)
        .map_err(|error| error.to_string())?;
    let companies = companies
        .into_iter()
        .filter(|company| company.sector.is_some() && (company.retained || company.in_universe))
        .collect::<Vec<_>>();
    let symbols: Vec<String> = companies
        .iter()
        .map(|company| company.symbol.clone())
        .collect();
    if symbols.is_empty() {
        return Err("the SEC universe catalog is empty".to_owned());
    }
    send_progress(
        events,
        SyncPhase::Snapshots,
        0,
        symbols.len(),
        "Refreshing current prices",
        None,
    );
    let snapshots = provider
        .fetch_snapshots(&symbols)
        .await
        .map_err(|error| error.to_string())?;
    storage
        .upsert_snapshots(&snapshots)
        .map_err(|error| error.to_string())?;
    let by_symbol: HashMap<&str, f64> = snapshots
        .iter()
        .filter_map(|snapshot| {
            snapshot
                .price
                .map(|price| (snapshot.symbol.as_str(), price))
        })
        .collect();
    let updated_companies: Vec<Company> = companies
        .into_iter()
        .map(|mut company| {
            company.market_cap = None;
            if let (Some(shares), Some(price)) = (
                company.shares_outstanding,
                by_symbol.get(company.symbol.as_str()),
            ) {
                let market_cap = shares * price;
                if market_cap.is_finite() && market_cap > 0.0 {
                    company.market_cap = Some(market_cap);
                    company.updated_at = Utc::now();
                }
            }
            company
        })
        .collect();
    storage
        .upsert_companies(&updated_companies)
        .map_err(|error| error.to_string())?;
    for sector in Sector::ALL {
        let candidates = updated_companies
            .iter()
            .filter(|company| company.sector == Some(sector))
            .cloned()
            .collect::<Vec<_>>();
        storage
            .replace_memberships(Utc::now().date_naive(), sector, &candidates)
            .map_err(|error| error.to_string())?;
    }
    storage
        .set_sync_checkpoint("snapshots", Utc::now())
        .map_err(|error| error.to_string())?;
    send_progress(
        events,
        SyncPhase::Snapshots,
        symbols.len(),
        symbols.len(),
        "Current prices cached",
        None,
    );
    let _ = events.send(SyncEvent::DataChanged);
    Ok(())
}

async fn backfill_history(
    storage: Storage,
    provider: Arc<AlpacaProvider>,
    events: mpsc::UnboundedSender<SyncEvent>,
    cancellation: CancellationToken,
    batch_size: usize,
) {
    let companies = match storage.companies(None, true) {
        Ok(companies) => companies,
        Err(error) => {
            let _ = events.send(SyncEvent::Error(error.to_string()));
            return;
        }
    };
    let total = companies.len();
    let now = provider.latest_historical_end(Utc::now());
    let mut failed_batches = 0_usize;
    let mut completed_symbols = 0_usize;
    for (batch_index, batch) in companies.chunks(batch_size.max(1)).enumerate() {
        if cancellation.is_cancelled() {
            return;
        }
        let symbols: Vec<String> = batch.iter().map(|company| company.symbol.clone()).collect();
        let watermarks = match batch
            .iter()
            .map(|company| storage.latest_bar_timestamp(&company.symbol, "1Day"))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(watermarks) => watermarks,
            Err(error) => {
                failed_batches += 1;
                let _ = events.send(SyncEvent::Error(error.to_string()));
                continue;
            }
        };
        let start = if watermarks.iter().any(Option::is_none) {
            now - ChronoDuration::days(1_826)
        } else {
            watermarks
                .into_iter()
                .flatten()
                .min()
                .map_or(now - ChronoDuration::days(1_826), |watermark| {
                    watermark - ChronoDuration::days(7)
                })
        };
        send_progress(
            &events,
            SyncPhase::History,
            (batch_index * batch_size).min(total),
            total,
            "Caching adjusted daily history",
            None,
        );
        match provider.fetch_bars(&symbols, "1Day", start, now).await {
            Ok(bars) => {
                if let Err(error) = storage.upsert_bars(&bars) {
                    failed_batches += 1;
                    let _ = events.send(SyncEvent::Error(error.to_string()));
                } else {
                    completed_symbols += batch.len();
                    let _ = events.send(SyncEvent::DataChanged);
                }
            }
            Err(error) => {
                failed_batches += 1;
                let _ = events.send(SyncEvent::Error(error.to_string()));
            }
        }
    }
    if failed_batches == 0 {
        let _ = storage.set_sync_checkpoint("history:1Day:all", Utc::now());
        send_progress(
            &events,
            SyncPhase::Complete,
            total,
            total,
            "Historical cache is current",
            None,
        );
    } else {
        send_progress(
            &events,
            SyncPhase::Error,
            completed_symbols,
            total,
            "Historical cache is incomplete; refresh to retry",
            Some(format!("{failed_batches} history batches failed")),
        );
    }
}

async fn refresh_ticker(
    storage: &Storage,
    provider: &AlpacaProvider,
    events: &mpsc::UnboundedSender<SyncEvent>,
    symbol: &str,
    range: DateRange,
) -> Result<(), String> {
    let now = Utc::now();
    let history_end = provider.latest_historical_end(now);
    send_progress(
        events,
        SyncPhase::News,
        0,
        3,
        &format!("Updating {symbol}"),
        None,
    );
    let symbols = vec![symbol.to_owned()];
    let (bars, news, snapshots) = tokio::join!(
        provider.fetch_bars(
            &symbols,
            range.preferred_timeframe(),
            range.cutoff(history_end),
            history_end,
        ),
        provider.fetch_news(&symbols, 20),
        provider.fetch_snapshots(&symbols),
    );
    let mut errors = Vec::new();
    match bars {
        Ok(bars) => {
            storage
                .upsert_bars(&bars)
                .map_err(|error| error.to_string())?;
        }
        Err(error) => errors.push(format!("bars: {error}")),
    }
    match news {
        Ok(news) => {
            storage
                .upsert_news(&news)
                .map_err(|error| error.to_string())?;
        }
        Err(error) => errors.push(format!("news: {error}")),
    }
    match snapshots {
        Ok(snapshots) => {
            storage
                .upsert_snapshots(&snapshots)
                .map_err(|error| error.to_string())?;
        }
        Err(error) => errors.push(format!("snapshot: {error}")),
    }
    let _ = events.send(SyncEvent::TickerChanged(symbol.to_owned()));
    if !errors.is_empty() {
        return Err(format!(
            "{symbol} update incomplete ({})",
            errors.join("; ")
        ));
    }
    send_progress(
        events,
        SyncPhase::Complete,
        3,
        3,
        &format!("{symbol} detail cached"),
        None,
    );
    Ok(())
}

async fn refresh_assets(
    storage: Storage,
    provider: Arc<AlpacaProvider>,
    events: mpsc::UnboundedSender<SyncEvent>,
) {
    let assets = match provider.fetch_assets().await {
        Ok(assets) => assets,
        Err(error) => {
            let _ = events.send(SyncEvent::Error(error.to_string()));
            return;
        }
    };
    let existing: HashMap<String, Company> = match storage.companies(None, false) {
        Ok(companies) => companies,
        Err(error) => {
            let _ = events.send(SyncEvent::Error(error.to_string()));
            return;
        }
    }
    .into_iter()
    .map(|company| (company.symbol.clone(), company))
    .collect();
    let merged: Vec<Company> = assets
        .into_iter()
        .map(|mut asset| {
            if let Some(current) = existing.get(&asset.symbol) {
                asset.sector = current.sector;
                asset.raw_sector.clone_from(&current.raw_sector);
                asset.industry.clone_from(&current.industry);
                asset.market_cap = current.market_cap;
                asset.shares_outstanding = current.shares_outstanding;
                asset.rank = current.rank;
                asset.description.clone_from(&current.description);
                asset.in_universe = current.in_universe;
                asset.retained = current.retained;
            }
            asset
        })
        .collect();
    if let Err(error) = storage.upsert_companies(&merged) {
        let _ = events.send(SyncEvent::Error(error.to_string()));
    }
}

fn send_progress(
    events: &mpsc::UnboundedSender<SyncEvent>,
    phase: SyncPhase,
    completed: usize,
    total: usize,
    message: &str,
    last_error: Option<String>,
) {
    let _ = events.send(SyncEvent::Progress(SyncProgress {
        phase,
        completed,
        total,
        message: message.to_owned(),
        last_error,
        updated_at: Utc::now(),
    }));
}

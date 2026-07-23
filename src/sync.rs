use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

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

#[derive(Debug, Clone, Copy)]
struct HistoryPlan {
    timeframe: &'static str,
    range: DateRange,
    checkpoint: &'static str,
    message: &'static str,
}

const HISTORY_PLANS: [HistoryPlan; 2] = [
    HistoryPlan {
        timeframe: "1Day",
        range: DateRange::TwoYears,
        checkpoint: "history:1Day:2Y",
        message: "Caching adjusted daily history",
    },
    HistoryPlan {
        timeframe: "1Week",
        range: DateRange::All,
        checkpoint: "history:1Week:all",
        message: "Caching all available weekly history",
    },
];

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
    if let Err(error) = refresh_assets(&storage, provider.as_ref(), &events).await {
        tracing::warn!(error = %error, "initial active-asset refresh failed");
        let _ = events.send(SyncEvent::Error(error));
    }
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
        .filter(is_snapshot_candidate)
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
    let total = companies.len().saturating_mul(HISTORY_PLANS.len());
    let now = provider.latest_historical_end(Utc::now());
    let mut failed_batches = 0_usize;
    let mut completed_units = 0_usize;
    for (plan_index, plan) in HISTORY_PLANS.into_iter().enumerate() {
        let completed_scopes = match storage.sync_checkpoint_scopes(plan.checkpoint) {
            Ok(scopes) => scopes,
            Err(error) => {
                let _ = events.send(SyncEvent::Error(error.to_string()));
                return;
            }
        };
        let failures_before_plan = failed_batches;
        for (batch_index, batch) in companies.chunks(batch_size.max(1)).enumerate() {
            if cancellation.is_cancelled() {
                return;
            }
            let symbols: Vec<String> = batch.iter().map(|company| company.symbol.clone()).collect();
            let checkpoint_scopes = symbols
                .iter()
                .map(|symbol| history_symbol_checkpoint(plan, symbol))
                .collect::<Vec<_>>();
            let fully_backfilled = checkpoint_scopes
                .iter()
                .all(|scope| completed_scopes.contains(scope));
            let watermarks = match batch
                .iter()
                .map(|company| storage.latest_bar_timestamp(&company.symbol, plan.timeframe))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(watermarks) => watermarks,
                Err(error) => {
                    failed_batches += 1;
                    let _ = events.send(SyncEvent::Error(error.to_string()));
                    continue;
                }
            };
            let start = history_batch_start(plan, now, &watermarks, fully_backfilled);
            send_progress(
                &events,
                SyncPhase::History,
                (plan_index * companies.len() + batch_index * batch_size).min(total),
                total,
                plan.message,
                None,
            );
            match provider
                .fetch_bars(&symbols, plan.timeframe, start, now)
                .await
            {
                Ok(bars) => {
                    let result = storage
                        .upsert_bars(&bars)
                        .and_then(|_| storage.set_sync_checkpoints(&checkpoint_scopes, Utc::now()));
                    if let Err(error) = result {
                        failed_batches += 1;
                        let _ = events.send(SyncEvent::Error(error.to_string()));
                    } else {
                        completed_units += batch.len();
                        let _ = events.send(SyncEvent::DataChanged);
                    }
                }
                Err(error) => {
                    failed_batches += 1;
                    let _ = events.send(SyncEvent::Error(error.to_string()));
                }
            }
        }
        if failed_batches == failures_before_plan {
            let _ = storage.set_sync_checkpoint(plan.checkpoint, Utc::now());
        }
    }
    if failed_batches == 0 {
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
            completed_units,
            total,
            "Historical cache is incomplete; refresh to retry",
            Some(format!("{failed_batches} history batches failed")),
        );
    }
}

fn history_symbol_checkpoint(plan: HistoryPlan, symbol: &str) -> String {
    format!("{}:symbol:{symbol}", plan.checkpoint)
}

fn history_batch_start(
    plan: HistoryPlan,
    now: chrono::DateTime<Utc>,
    watermarks: &[Option<chrono::DateTime<Utc>>],
    fully_backfilled: bool,
) -> chrono::DateTime<Utc> {
    if !fully_backfilled || watermarks.iter().any(Option::is_none) {
        return plan.range.cutoff(now);
    }
    watermarks.iter().flatten().min().map_or_else(
        || plan.range.cutoff(now),
        |watermark| *watermark - ChronoDuration::days(7),
    )
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
    storage: &Storage,
    provider: &AlpacaProvider,
    events: &mpsc::UnboundedSender<SyncEvent>,
) -> Result<(), String> {
    send_progress(
        events,
        SyncPhase::Universe,
        0,
        1,
        "Checking active Alpaca assets",
        None,
    );
    let assets = provider
        .fetch_assets()
        .await
        .map_err(|error| error.to_string())?;
    let existing: HashMap<String, Company> = storage
        .companies(None, false)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|company| (company.symbol.clone(), company))
        .collect();
    let merged = reconcile_active_assets(assets, &existing, Utc::now());
    storage
        .upsert_companies(&merged)
        .map_err(|error| error.to_string())?;
    let as_of = Utc::now().date_naive();
    for sector in Sector::ALL {
        let candidates = merged
            .iter()
            .filter(|company| company.sector == Some(sector) && company.retained)
            .cloned()
            .collect::<Vec<_>>();
        storage
            .replace_memberships(as_of, sector, &candidates)
            .map_err(|error| error.to_string())?;
    }
    send_progress(
        events,
        SyncPhase::Universe,
        1,
        1,
        "Active asset catalog reconciled",
        None,
    );
    let _ = events.send(SyncEvent::DataChanged);
    Ok(())
}

fn reconcile_active_assets(
    active_assets: Vec<Company>,
    existing: &HashMap<String, Company>,
    updated_at: chrono::DateTime<Utc>,
) -> Vec<Company> {
    let active_symbols = active_assets
        .iter()
        .map(|asset| asset.symbol.clone())
        .collect::<HashSet<_>>();
    let mut merged = active_assets
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
                asset.retained = current.sector.is_some() || current.retained;
            }
            asset
        })
        .collect::<Vec<_>>();
    for current in existing.values().filter(|company| {
        company.sector.is_some() && !active_symbols.contains(company.symbol.as_str())
    }) {
        let mut inactive = current.clone();
        inactive.in_universe = false;
        inactive.retained = false;
        inactive.updated_at = updated_at;
        merged.push(inactive);
    }
    merged
}

fn is_snapshot_candidate(company: &Company) -> bool {
    company.retained || company.in_universe
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

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;

    fn instant() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 23, 12, 0, 0)
            .single()
            .expect("valid fixture timestamp")
    }

    fn company(symbol: &str, sector: Option<Sector>, retained: bool, in_universe: bool) -> Company {
        Company {
            symbol: symbol.to_owned(),
            name: format!("{symbol} name"),
            sector,
            raw_sector: sector.map(|value| value.label().to_owned()),
            exchange: "TEST".to_owned(),
            industry: "Test industry".to_owned(),
            market_cap: Some(1_000_000.0),
            shares_outstanding: Some(10_000.0),
            rank: Some(1),
            description: "Catalog description".to_owned(),
            in_universe,
            retained,
            updated_at: instant() - Duration::days(1),
        }
    }

    #[test]
    fn active_assets_reactivate_catalog_candidates_and_preserve_non_catalog_flags() {
        let active_catalog = company("LIVE", Some(Sector::Technology), false, false);
        let inactive_catalog = company("GONE", Some(Sector::Technology), true, true);
        let benchmark = company("SPY", None, true, true);
        let searchable = company("FUND", None, false, false);
        let existing = [active_catalog, inactive_catalog, benchmark, searchable]
            .into_iter()
            .map(|company| (company.symbol.clone(), company))
            .collect::<HashMap<_, _>>();
        let active = ["LIVE", "SPY", "FUND"]
            .into_iter()
            .map(|symbol| company(symbol, None, false, false))
            .collect();

        let reconciled = reconcile_active_assets(active, &existing, instant());
        let by_symbol = reconciled
            .iter()
            .map(|company| (company.symbol.as_str(), company))
            .collect::<HashMap<_, _>>();

        let live = by_symbol["LIVE"];
        assert_eq!(live.sector, Some(Sector::Technology));
        assert!(live.retained);
        assert!(!live.in_universe);
        assert_eq!(live.description, "Catalog description");

        let gone = by_symbol["GONE"];
        assert!(!gone.retained);
        assert!(!gone.in_universe);
        assert_eq!(gone.updated_at, instant());

        assert!(by_symbol["SPY"].retained);
        assert!(by_symbol["SPY"].in_universe);
        assert!(!by_symbol["FUND"].retained);
        assert!(!by_symbol["FUND"].in_universe);
    }

    #[test]
    fn snapshot_candidates_exclude_reconciled_inactive_catalog_rows() {
        assert!(is_snapshot_candidate(&company(
            "LIVE",
            Some(Sector::Technology),
            true,
            false
        )));
        assert!(is_snapshot_candidate(&company("SPY", None, true, true)));
        assert!(!is_snapshot_candidate(&company(
            "GONE",
            Some(Sector::Technology),
            false,
            false
        )));
    }

    #[test]
    fn history_plans_cover_medium_and_all_available_ranges() {
        assert_eq!(HISTORY_PLANS[0].timeframe, "1Day");
        assert_eq!(HISTORY_PLANS[0].range, DateRange::TwoYears);
        assert_eq!(HISTORY_PLANS[1].timeframe, "1Week");
        assert_eq!(HISTORY_PLANS[1].range, DateRange::All);

        let now = instant();
        assert_eq!(
            history_batch_start(HISTORY_PLANS[0], now, &[None], false),
            DateRange::TwoYears.cutoff(now)
        );
        assert_eq!(
            history_batch_start(HISTORY_PLANS[1], now, &[None], false),
            chrono::DateTime::UNIX_EPOCH
        );
    }

    #[test]
    fn completed_history_plan_resumes_with_a_bounded_overlap() {
        let now = instant();
        let first = now - Duration::days(4);
        let second = now - Duration::days(2);
        assert_eq!(
            history_batch_start(HISTORY_PLANS[1], now, &[Some(second), Some(first)], true),
            first - Duration::days(7)
        );
    }
}

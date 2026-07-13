use std::{fs, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

use crate::config::Settings;

pub struct LogGuard {
    _worker: WorkerGuard,
    pub path: PathBuf,
}

pub fn init(settings: &Settings) -> Result<LogGuard> {
    let log_dir = settings.cache_dir.join("logs");
    fs::create_dir_all(&log_dir).context("could not create log directory")?;
    let appender = tracing_appender::rolling::daily(&log_dir, "stock-tui.log");
    let (writer, worker) = tracing_appender::non_blocking(appender);
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("stock_tui=info,warn"));
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(filter)
        .with_writer(writer)
        .try_init()
        .map_err(|error| anyhow!("could not initialize application logging: {error}"))?;
    Ok(LogGuard {
        _worker: worker,
        path: log_dir,
    })
}

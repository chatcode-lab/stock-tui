use anyhow::Result;
use clap::Parser;
use stock_tui::{cli::Cli, config::Settings, logging, runtime};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let settings = Settings::load(&cli)?;

    if cli.print_config {
        println!("{settings:#?}");
        return Ok(());
    }

    let _log_guard = logging::init(&settings)?;
    runtime::run(settings).await
}

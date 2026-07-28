use std::io::{self, Write};

use anyhow::Result;
use clap::Parser;
use stock_tui::{cli::Cli, config::Settings, logging, onboarding::OnboardingOutcome, runtime};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut settings = Settings::load(&cli)?;

    if cli.print_config {
        println!("{settings:#?}");
        return Ok(());
    }

    if stock_tui::onboarding::ensure_ready(&mut settings).await? == OnboardingOutcome::Demo {
        let mut demo_cli = cli.clone();
        demo_cli.demo = true;
        settings = Settings::load(&demo_cli)?;
    }
    let cache_kind = if settings.demo { "demo" } else { "market" };
    println!("Preparing the local {cache_kind} cache and starting the terminal UI...");
    io::stdout().flush()?;
    let _log_guard = logging::init(&settings)?;
    runtime::run(settings).await
}

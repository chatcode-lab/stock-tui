use std::{
    env,
    io::{self, IsTerminal, Write},
};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use secrecy::{ExposeSecret, SecretString};

use crate::{
    config::{CredentialSource, Credentials, Settings},
    credentials,
    providers::{AlpacaProvider, ProviderError},
    terminal::{copy_to_terminal_clipboard, terminal_hyperlink_sequence},
};

pub const ALPACA_SIGNUP_URL: &str = "https://app.alpaca.markets/signup";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingOutcome {
    Ready,
    Demo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegistrationAction {
    Open,
    Copy,
    Demo,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegistrationDelivery {
    Opened,
    Copied,
    VisibleOnly,
    Skipped,
}

pub async fn ensure_ready(settings: &mut Settings) -> Result<OnboardingOutcome> {
    if settings.demo || settings.offline {
        return Ok(OnboardingOutcome::Ready);
    }
    if !settings.provider.requires_credentials() {
        return Ok(OnboardingOutcome::Ready);
    }
    if settings.incomplete_environment_credentials {
        eprintln!(
            "Ignoring an incomplete Alpaca environment pair; set both \
             ALPACA_API_KEY and ALPACA_API_SECRET or remove both."
        );
    }

    if let Some(existing) = settings.credentials.clone() {
        print_status("Checking configured Alpaca credentials...")?;
        match validate(settings, &existing).await {
            Ok(()) => return Ok(OnboardingOutcome::Ready),
            Err(ProviderError::Authentication) => {
                eprintln!(
                    "The configured Alpaca credentials were rejected by the Paper Trading API."
                );
                if settings.credential_source == Some(CredentialSource::Environment)
                    && try_stored_fallback(settings, &existing).await?
                {
                    return Ok(OnboardingOutcome::Ready);
                }
            }
            Err(error) => {
                eprintln!("Alpaca credential preflight was unavailable: {error}");
                eprintln!("Continuing with the local cache; synchronization will retry.");
                return Ok(OnboardingOutcome::Ready);
            }
        }
    }

    run_prompt(settings).await
}

async fn try_stored_fallback(settings: &mut Settings, rejected: &Credentials) -> Result<bool> {
    let stored = match credentials::load(&settings.credentials_path()) {
        Ok(Some(stored)) => stored,
        Ok(None) => return Ok(false),
        Err(error) => {
            eprintln!("Ignoring unusable stored Alpaca credentials: {error}");
            return Ok(false);
        }
    };
    if credentials_match(&stored, rejected) {
        return Ok(false);
    }
    print_status("Checking saved Alpaca credentials...")?;
    match validate(settings, &stored).await {
        Ok(()) => {
            settings.set_credentials(stored, CredentialSource::StoredFile);
            eprintln!(
                "Using the saved credentials instead. Remove or update the rejected \
                 ALPACA_API_KEY and ALPACA_API_SECRET environment values."
            );
            Ok(true)
        }
        Err(ProviderError::Authentication) => Ok(false),
        Err(error) => {
            settings.set_credentials(stored, CredentialSource::StoredFile);
            eprintln!("Saved-credential preflight was unavailable: {error}");
            eprintln!("Continuing with the local cache; synchronization will retry.");
            Ok(true)
        }
    }
}

async fn run_prompt(settings: &mut Settings) -> Result<OnboardingOutcome> {
    println!();
    println!("stock-tui needs a personal Alpaca Paper Trading API key.");
    println!("Create a free account or generate Paper Trading keys here:");
    println!(
        "{}",
        registration_url_for_output(
            io::stdout().is_terminal(),
            env::var_os("NO_COLOR").is_none()
        )
    );
    let action = prompt_registration_action()?;
    if action == RegistrationAction::Demo {
        println!("Starting demo mode without credentials.");
        return Ok(OnboardingOutcome::Demo);
    }
    match deliver_registration_link(
        action,
        |url| webbrowser::open(url).map_err(|_| ()),
        copy_to_terminal_clipboard,
    ) {
        RegistrationDelivery::Opened => println!("Opened the Alpaca registration page."),
        RegistrationDelivery::Copied => {
            println!("The registration URL was sent to the terminal clipboard.")
        }
        RegistrationDelivery::VisibleOnly => {
            println!("Could not open or copy the registration URL; use the URL shown above.")
        }
        RegistrationDelivery::Skipped => println!("Continuing without opening the link."),
    }
    println!("Input is hidden. Press Ctrl-C to cancel.");

    loop {
        let candidate = prompt_credentials_with(|label| rpassword::prompt_password(label))?;
        print_status("Validating Alpaca credentials...")?;
        match validate(settings, &candidate).await {
            Ok(()) => {
                let path = settings.credentials_path();
                let source = match credentials::save(&path, &candidate) {
                    Ok(()) => {
                        println!("Credentials validated and saved at {}.", path.display());
                        CredentialSource::StoredFile
                    }
                    Err(error) => {
                        eprintln!("Credentials are valid but could not be saved: {error}");
                        eprintln!("They will be used for this session only.");
                        CredentialSource::OnboardingSession
                    }
                };
                if settings.credential_source == Some(CredentialSource::Environment) {
                    eprintln!(
                        "Remove or update the rejected environment values before the next launch."
                    );
                }
                settings.set_credentials(candidate, source);
                return Ok(OnboardingOutcome::Ready);
            }
            Err(ProviderError::Authentication) => {
                eprintln!("Alpaca rejected that key pair. Enter Paper Trading credentials again.");
            }
            Err(ProviderError::Permission { .. }) => {
                eprintln!(
                    "That account cannot access the configured Paper Trading endpoint. \
                     Check the account selector and enter a new pair."
                );
            }
            Err(error) => {
                return Err(error).context(
                    "could not validate new Alpaca credentials; no credentials were saved",
                );
            }
        }
    }
}

fn print_status(message: &str) -> Result<()> {
    println!("{message}");
    io::stdout()
        .flush()
        .context("could not display startup status")
}

fn registration_url_for_output(terminal: bool, color: bool) -> String {
    if terminal {
        terminal_hyperlink_sequence(ALPACA_SIGNUP_URL, color)
            .unwrap_or_else(|| ALPACA_SIGNUP_URL.to_owned())
    } else {
        ALPACA_SIGNUP_URL.to_owned()
    }
}

async fn validate(settings: &Settings, credentials: &Credentials) -> Result<(), ProviderError> {
    let mut candidate = settings.clone();
    candidate.credentials = Some(credentials.clone());
    AlpacaProvider::new(&candidate)?
        .validate_credentials()
        .await
}

fn prompt_registration_action() -> Result<RegistrationAction> {
    print!(
        "Press Enter to open, c to copy, d for demo, \
         or Esc to continue to key entry: "
    );
    io::stdout()
        .flush()
        .context("could not display the registration prompt")?;

    let action = (|| {
        let _raw_mode = RawModeGuard::enter()?;
        read_registration_action_with(event::read)
    })();
    println!();

    action.context("could not read the registration choice")
}

fn read_registration_action_with(
    mut read: impl FnMut() -> io::Result<Event>,
) -> io::Result<RegistrationAction> {
    loop {
        let Event::Key(key) = read()? else {
            continue;
        };
        if let Some(action) = registration_action_for_key(key)? {
            return Ok(action);
        }
    }
}

fn registration_action_for_key(key: KeyEvent) -> io::Result<Option<RegistrationAction>> {
    if key.kind == KeyEventKind::Release {
        return Ok(None);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'C'))
    {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "registration prompt cancelled",
        ));
    }

    let action = match key.code {
        KeyCode::Enter => Some(RegistrationAction::Open),
        KeyCode::Char('c' | 'C')
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            Some(RegistrationAction::Copy)
        }
        KeyCode::Char('d' | 'D')
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            Some(RegistrationAction::Demo)
        }
        KeyCode::Esc => Some(RegistrationAction::Skip),
        _ => None,
    };
    Ok(action)
}

fn credentials_match(left: &Credentials, right: &Credentials) -> bool {
    left.key.expose_secret() == right.key.expose_secret()
        && left.secret.expose_secret() == right.secret.expose_secret()
}

fn prompt_credentials_with(
    mut prompt: impl FnMut(&str) -> io::Result<String>,
) -> Result<Credentials> {
    let key = prompt_nonempty(&mut prompt, "Alpaca API key ID (hidden): ")?;
    let secret = prompt_nonempty(&mut prompt, "Alpaca API secret (hidden): ")?;
    Ok(Credentials {
        key: SecretString::from(key),
        secret: SecretString::from(secret),
    })
}

fn prompt_nonempty(
    prompt: &mut impl FnMut(&str) -> io::Result<String>,
    label: &'static str,
) -> Result<String> {
    loop {
        let value = prompt(label).context("could not read hidden credential input")?;
        if !value.is_empty() && value.trim() == value {
            return Ok(value);
        }
        eprintln!("The value cannot be empty or start/end with whitespace.");
    }
}

fn deliver_registration_link(
    action: RegistrationAction,
    mut open: impl FnMut(&str) -> Result<(), ()>,
    mut copy: impl FnMut(&str) -> io::Result<()>,
) -> RegistrationDelivery {
    match action {
        RegistrationAction::Open if open(ALPACA_SIGNUP_URL).is_ok() => RegistrationDelivery::Opened,
        RegistrationAction::Open | RegistrationAction::Copy if copy(ALPACA_SIGNUP_URL).is_ok() => {
            RegistrationDelivery::Copied
        }
        RegistrationAction::Open | RegistrationAction::Copy => RegistrationDelivery::VisibleOnly,
        RegistrationAction::Demo | RegistrationAction::Skip => RegistrationDelivery::Skipped,
    }
}

struct RawModeGuard;

impl RawModeGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque, path::PathBuf, time::Duration};

    use super::*;
    use crate::config::{DEFAULT_CATALOG_URL, DEFAULT_STOCK_API_URL, ProviderKind};

    #[tokio::test]
    async fn stock_api_never_enters_alpaca_credential_onboarding() {
        let mut settings = Settings {
            credentials: None,
            credential_source: None,
            incomplete_environment_credentials: true,
            db_path: PathBuf::from("market.sqlite3"),
            config_dir: PathBuf::from("config"),
            cache_dir: PathBuf::from("cache"),
            provider: ProviderKind::StockApi,
            catalog_url: DEFAULT_CATALOG_URL.to_owned(),
            catalog_refresh_interval: Duration::from_secs(12 * 60 * 60),
            stock_api_url: DEFAULT_STOCK_API_URL.to_owned(),
            stock_api_news: true,
            stock_api_token: None,
            data_url: String::new(),
            trading_url: String::new(),
            feed: "managed".to_owned(),
            refresh_interval: Duration::from_secs(300),
            request_limit_per_minute: 180,
            snapshot_batch_size: 100,
            history_batch_size: 50,
            demo: false,
            offline: false,
            reset_demo: false,
        };

        assert_eq!(
            ensure_ready(&mut settings)
                .await
                .expect("onboarding result"),
            OnboardingOutcome::Ready
        );
        assert!(settings.credentials.is_none());
    }

    #[test]
    fn registration_prefers_browser_and_copies_only_after_failure() {
        let copied = RefCell::new(Vec::new());
        let opened = deliver_registration_link(
            RegistrationAction::Open,
            |_| Ok(()),
            |url| {
                copied.borrow_mut().push(url.to_owned());
                Ok(())
            },
        );
        assert_eq!(opened, RegistrationDelivery::Opened);
        assert!(copied.borrow().is_empty());

        let copied_delivery = deliver_registration_link(
            RegistrationAction::Open,
            |_| Err(()),
            |url| {
                copied.borrow_mut().push(url.to_owned());
                Ok(())
            },
        );
        assert_eq!(copied_delivery, RegistrationDelivery::Copied);
        assert_eq!(copied.borrow().as_slice(), [ALPACA_SIGNUP_URL]);
    }

    #[test]
    fn registration_url_is_highlighted_only_for_terminal_output() {
        assert_eq!(registration_url_for_output(false, true), ALPACA_SIGNUP_URL);

        let highlighted = registration_url_for_output(true, true);
        assert!(highlighted.starts_with("\u{1b}]8;;https://"));
        assert!(highlighted.contains("\u{1b}[1;4;96m"));
        assert!(highlighted.contains(ALPACA_SIGNUP_URL));
        assert!(highlighted.ends_with("\u{1b}]8;;\u{1b}\\"));
    }

    #[test]
    fn registration_remains_visible_when_integrations_fail() {
        assert_eq!(
            deliver_registration_link(
                RegistrationAction::Open,
                |_| Err(()),
                |_| Err(io::Error::other("clipboard unavailable"))
            ),
            RegistrationDelivery::VisibleOnly
        );
    }

    #[test]
    fn copy_and_skip_never_open_the_browser() {
        let opened = RefCell::new(Vec::new());
        let copied = RefCell::new(Vec::new());
        assert_eq!(
            deliver_registration_link(
                RegistrationAction::Copy,
                |url| {
                    opened.borrow_mut().push(url.to_owned());
                    Ok(())
                },
                |url| {
                    copied.borrow_mut().push(url.to_owned());
                    Ok(())
                }
            ),
            RegistrationDelivery::Copied
        );
        assert!(opened.borrow().is_empty());
        assert_eq!(copied.borrow().as_slice(), [ALPACA_SIGNUP_URL]);

        copied.borrow_mut().clear();
        assert_eq!(
            deliver_registration_link(
                RegistrationAction::Skip,
                |url| {
                    opened.borrow_mut().push(url.to_owned());
                    Ok(())
                },
                |url| {
                    copied.borrow_mut().push(url.to_owned());
                    Ok(())
                }
            ),
            RegistrationDelivery::Skipped
        );
        assert!(opened.borrow().is_empty());
        assert!(copied.borrow().is_empty());
    }

    #[test]
    fn registration_keys_map_to_actions_and_ctrl_c_cancels() {
        assert_eq!(
            registration_action_for_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
                .expect("enter"),
            Some(RegistrationAction::Open)
        );
        assert_eq!(
            registration_action_for_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
                .expect("copy"),
            Some(RegistrationAction::Copy)
        );
        assert_eq!(
            registration_action_for_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
                .expect("demo"),
            Some(RegistrationAction::Demo)
        );
        assert_eq!(
            registration_action_for_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
                .expect("escape"),
            Some(RegistrationAction::Skip)
        );
        assert_eq!(
            registration_action_for_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
                .expect("unknown key"),
            None
        );
        assert_eq!(
            registration_action_for_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
                .expect_err("control-c must cancel")
                .kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn registration_reader_ignores_unrelated_events() {
        let mut events = VecDeque::from([
            Event::Resize(120, 40),
            Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
            Event::Key(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::SHIFT)),
        ]);
        assert_eq!(
            read_registration_action_with(|| {
                events
                    .pop_front()
                    .ok_or_else(|| io::Error::other("no scripted event"))
            })
            .expect("registration action"),
            RegistrationAction::Copy
        );
    }

    #[test]
    fn hidden_prompt_rejects_blank_values_and_never_formats_secrets() {
        let mut values = VecDeque::from([
            String::new(),
            " fixture-key ".to_owned(),
            "fixture-key".to_owned(),
            "fixture-secret".to_owned(),
        ]);
        let credentials = prompt_credentials_with(|_| {
            values
                .pop_front()
                .ok_or_else(|| io::Error::other("no scripted input"))
        })
        .expect("prompted credentials");
        assert_eq!(credentials.key.expose_secret(), "fixture-key");
        assert_eq!(credentials.secret.expose_secret(), "fixture-secret");
        let debug = format!("{credentials:?}");
        assert!(!debug.contains("fixture-key"));
        assert!(!debug.contains("fixture-secret"));
    }

    #[test]
    fn credential_comparison_requires_the_complete_pair() {
        let first = Credentials {
            key: SecretString::from("key".to_owned()),
            secret: SecretString::from("secret".to_owned()),
        };
        let different = Credentials {
            key: SecretString::from("key".to_owned()),
            secret: SecretString::from("other".to_owned()),
        };
        assert!(credentials_match(&first, &first));
        assert!(!credentials_match(&first, &different));
    }
}

use std::{
    fmt,
    io::{self, Stdout, Write, stdout},
    panic,
    sync::{
        Once,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::fs::OpenOptions;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use crossterm::{
    Command,
    cursor::Show,
    event::{
        DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange, poll,
        read,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

#[cfg(windows)]
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};

pub type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

const INPUT_QUIET_PERIOD: Duration = Duration::from_millis(40);
const INPUT_DRAIN_LIMIT: Duration = Duration::from_millis(200);

static INSTALL_PANIC_HOOK: Once = Once::new();
static TERMINAL_ACTIVE: AtomicBool = AtomicBool::new(false);

pub struct TerminalSession {
    terminal: TuiTerminal,
    input_modes_disabled: bool,
    restored: bool,
}

impl TerminalSession {
    pub fn start() -> io::Result<Self> {
        install_panic_restore_hook();
        enable_raw_mode()?;
        TERMINAL_ACTIVE.store(true, Ordering::SeqCst);
        let setup = (|| {
            execute!(
                stdout(),
                EnterAlternateScreen,
                EnableSgrMouseCapture,
                EnableFocusChange,
                EnableBracketedPaste
            )?;
            let backend = CrosstermBackend::new(stdout());
            let terminal = Terminal::new(backend)?;
            Ok(Self {
                terminal,
                input_modes_disabled: false,
                restored: false,
            })
        })();
        if setup.is_err() {
            let outcome = execute_cleanup_actions(IMMEDIATE_RESTORE_ACTIONS);
            if outcome.raw_mode_disabled {
                TERMINAL_ACTIVE.store(false, Ordering::SeqCst);
            }
        }
        setup
    }

    pub fn terminal_mut(&mut self) -> &mut TuiTerminal {
        &mut self.terminal
    }

    pub fn disable_input_modes(&mut self) -> io::Result<()> {
        if self.input_modes_disabled {
            return Ok(());
        }
        let outcome = execute_cleanup_actions(DISABLE_INPUT_ACTIONS);
        if outcome.first_error.is_none() {
            self.input_modes_disabled = true;
        }
        outcome.into_result()
    }

    pub fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }

        let mut first_error = self.disable_input_modes().err();
        record_first_error(&mut first_error, drain_pending_input());
        let outcome = execute_cleanup_actions(FINISH_RESTORE_ACTIONS);
        let raw_mode_disabled = outcome.raw_mode_disabled;
        if first_error.is_none() {
            first_error = outcome.first_error;
        }
        if raw_mode_disabled {
            TERMINAL_ACTIVE.store(false, Ordering::SeqCst);
        }
        self.restored = raw_mode_disabled && first_error.is_none();
        first_error.map_or(Ok(()), Err)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if self.restored {
            return;
        }
        let outcome = execute_cleanup_actions(IMMEDIATE_RESTORE_ACTIONS);
        if outcome.raw_mode_disabled {
            self.restored = true;
            TERMINAL_ACTIVE.store(false, Ordering::SeqCst);
        }
    }
}

pub(crate) fn copy_to_terminal_clipboard(value: &str) -> io::Result<()> {
    let mut output = stdout().lock();
    output.write_all(terminal_clipboard_sequence(value).as_bytes())?;
    output.flush()
}

pub(crate) fn terminal_clipboard_sequence(value: &str) -> String {
    format!("\x1b]52;c;{}\x1b\\", STANDARD.encode(value))
}

pub(crate) fn terminal_hyperlink_sequence(value: &str, color: bool) -> Option<String> {
    if value.chars().any(char::is_control) {
        return None;
    }
    let style = if color { "\x1b[1;4;96m" } else { "\x1b[4m" };
    Some(format!(
        "\x1b]8;;{value}\x1b\\{style}{value}\x1b[0m\
         \x1b]8;;\x1b\\"
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupAction {
    DisableBracketedPaste,
    DisableFocusChange,
    DisableSgrMouseCapture,
    FlushOutput,
    FlushInput,
    LeaveAlternateScreen,
    ShowCursor,
    DisableRawMode,
}

const DISABLE_INPUT_ACTIONS: &[CleanupAction] = &[
    CleanupAction::DisableBracketedPaste,
    CleanupAction::DisableFocusChange,
    CleanupAction::DisableSgrMouseCapture,
    CleanupAction::FlushOutput,
];

const FINISH_RESTORE_ACTIONS: &[CleanupAction] = &[
    CleanupAction::FlushInput,
    CleanupAction::LeaveAlternateScreen,
    CleanupAction::ShowCursor,
    CleanupAction::FlushOutput,
    CleanupAction::FlushInput,
    CleanupAction::DisableRawMode,
];

const IMMEDIATE_RESTORE_ACTIONS: &[CleanupAction] = &[
    CleanupAction::DisableBracketedPaste,
    CleanupAction::DisableFocusChange,
    CleanupAction::DisableSgrMouseCapture,
    CleanupAction::FlushOutput,
    CleanupAction::FlushInput,
    CleanupAction::LeaveAlternateScreen,
    CleanupAction::ShowCursor,
    CleanupAction::FlushOutput,
    CleanupAction::FlushInput,
    CleanupAction::DisableRawMode,
];

#[derive(Default)]
struct CleanupOutcome {
    first_error: Option<io::Error>,
    raw_mode_disabled: bool,
}

impl CleanupOutcome {
    fn into_result(self) -> io::Result<()> {
        self.first_error.map_or(Ok(()), Err)
    }
}

fn execute_cleanup_actions(actions: &[CleanupAction]) -> CleanupOutcome {
    execute_cleanup_actions_with(actions, execute_cleanup_action)
}

fn execute_cleanup_actions_with(
    actions: &[CleanupAction],
    mut execute_action: impl FnMut(CleanupAction) -> io::Result<()>,
) -> CleanupOutcome {
    let mut outcome = CleanupOutcome::default();
    for &action in actions {
        match execute_action(action) {
            Ok(()) if action == CleanupAction::DisableRawMode => {
                outcome.raw_mode_disabled = true;
            }
            Ok(()) => {}
            Err(error) => record_first_error(&mut outcome.first_error, Err(error)),
        }
    }
    outcome
}

fn execute_cleanup_action(action: CleanupAction) -> io::Result<()> {
    match action {
        CleanupAction::DisableBracketedPaste => {
            execute!(stdout(), DisableBracketedPaste)
        }
        CleanupAction::DisableFocusChange => execute!(stdout(), DisableFocusChange),
        CleanupAction::DisableSgrMouseCapture => {
            execute!(stdout(), DisableSgrMouseCapture)
        }
        CleanupAction::FlushOutput => stdout().flush(),
        CleanupAction::FlushInput => flush_terminal_input(),
        CleanupAction::LeaveAlternateScreen => execute!(stdout(), LeaveAlternateScreen),
        CleanupAction::ShowCursor => execute!(stdout(), Show),
        CleanupAction::DisableRawMode => disable_raw_mode(),
    }
}

fn drain_pending_input() -> io::Result<()> {
    let started = Instant::now();
    let deadline = started + INPUT_DRAIN_LIMIT;
    let mut quiet_deadline = started + INPUT_QUIET_PERIOD;

    loop {
        let now = Instant::now();
        if now >= deadline || now >= quiet_deadline {
            return Ok(());
        }
        let wait = deadline.min(quiet_deadline).saturating_duration_since(now);
        if poll(wait)? {
            let _ = read()?;
            quiet_deadline = Instant::now() + INPUT_QUIET_PERIOD;
        }
    }
}

#[cfg(unix)]
fn flush_terminal_input() -> io::Result<()> {
    use rustix::termios::{QueueSelector, isatty, tcflush};

    let input = io::stdin();
    if isatty(&input) {
        tcflush(&input, QueueSelector::IFlush)?;
        return Ok(());
    }

    let tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    tcflush(&tty, QueueSelector::IFlush)?;
    Ok(())
}

#[cfg(windows)]
fn flush_terminal_input() -> io::Result<()> {
    use crossterm_winapi::{Console, Handle, HandleType};

    let console = Console::from(Handle::new(HandleType::InputHandle)?);
    let _ = console.read_console_input()?;
    Ok(())
}

fn record_first_error(first_error: &mut Option<io::Error>, result: io::Result<()>) {
    if let Err(error) = result
        && first_error.is_none()
    {
        *first_error = Some(error);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnableSgrMouseCapture;

impl Command for EnableSgrMouseCapture {
    fn write_ansi(&self, output: &mut impl fmt::Write) -> fmt::Result {
        // Track hover as well as buttons, and force the text-only SGR encoding.
        output.write_str("\x1b[?1003h\x1b[?1006h")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        EnableMouseCapture.execute_winapi()
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        EnableMouseCapture.is_ansi_code_supported()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DisableSgrMouseCapture;

impl Command for DisableSgrMouseCapture {
    fn write_ansi(&self, output: &mut impl fmt::Write) -> fmt::Result {
        output.write_str("\x1b[?1006l\x1b[?1003l")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        DisableMouseCapture.execute_winapi()
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        DisableMouseCapture.is_ansi_code_supported()
    }
}

fn install_panic_restore_hook() {
    INSTALL_PANIC_HOOK.call_once(|| {
        let original = panic::take_hook();
        panic::set_hook(Box::new(move |panic_info| {
            if TERMINAL_ACTIVE.load(Ordering::SeqCst) {
                let outcome = execute_cleanup_actions(IMMEDIATE_RESTORE_ACTIONS);
                if outcome.raw_mode_disabled {
                    TERMINAL_ACTIVE.store(false, Ordering::SeqCst);
                }
            }
            original(panic_info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ansi(command: impl Command) -> String {
        let mut output = String::new();
        command
            .write_ansi(&mut output)
            .expect("writing ANSI to a string succeeds");
        output
    }

    #[test]
    fn mouse_capture_uses_only_sgr_encoding() {
        let enable = ansi(EnableSgrMouseCapture);

        assert_eq!(enable, "\x1b[?1003h\x1b[?1006h");
        assert!(!enable.contains("?1000h"));
        assert!(!enable.contains("?1002h"));
        assert!(!enable.contains("?1005h"));
        assert!(!enable.contains("?1015h"));
    }

    #[test]
    fn mouse_capture_cleanup_reverses_enabled_modes() {
        assert_eq!(ansi(DisableSgrMouseCapture), "\x1b[?1006l\x1b[?1003l");
    }

    #[test]
    fn normal_cleanup_disables_input_before_restoring_raw_mode() {
        let actions = DISABLE_INPUT_ACTIONS
            .iter()
            .chain(FINISH_RESTORE_ACTIONS)
            .copied()
            .collect::<Vec<_>>();

        assert_eq!(
            actions,
            [
                CleanupAction::DisableBracketedPaste,
                CleanupAction::DisableFocusChange,
                CleanupAction::DisableSgrMouseCapture,
                CleanupAction::FlushOutput,
                CleanupAction::FlushInput,
                CleanupAction::LeaveAlternateScreen,
                CleanupAction::ShowCursor,
                CleanupAction::FlushOutput,
                CleanupAction::FlushInput,
                CleanupAction::DisableRawMode,
            ]
        );
    }

    #[test]
    fn cleanup_attempts_every_action_and_preserves_the_first_error() {
        let mut attempted = Vec::new();
        let outcome = execute_cleanup_actions_with(IMMEDIATE_RESTORE_ACTIONS, |action| {
            attempted.push(action);
            match action {
                CleanupAction::DisableFocusChange => Err(io::Error::other("first cleanup failure")),
                CleanupAction::LeaveAlternateScreen => {
                    Err(io::Error::other("later cleanup failure"))
                }
                _ => Ok(()),
            }
        });

        assert_eq!(attempted, IMMEDIATE_RESTORE_ACTIONS);
        assert!(outcome.raw_mode_disabled);
        assert_eq!(
            outcome
                .first_error
                .expect("the first error is retained")
                .to_string(),
            "first cleanup failure"
        );
    }

    #[test]
    fn terminal_clipboard_uses_osc_52_with_a_base64_payload() {
        assert_eq!(
            terminal_clipboard_sequence("https://example.test/news?a=1&b=2"),
            "\u{1b}]52;c;aHR0cHM6Ly9leGFtcGxlLnRlc3QvbmV3cz9hPTEmYj0y\u{1b}\\"
        );
    }

    #[test]
    fn terminal_hyperlink_uses_osc_8_and_keeps_the_url_visible() {
        assert_eq!(
            terminal_hyperlink_sequence("https://example.test/signup", true)
                .expect("safe hyperlink"),
            "\u{1b}]8;;https://example.test/signup\u{1b}\\\
             \u{1b}[1;4;96mhttps://example.test/signup\u{1b}[0m\
             \u{1b}]8;;\u{1b}\\"
        );
        assert_eq!(
            terminal_hyperlink_sequence("https://example.test/signup", false)
                .expect("safe hyperlink"),
            "\u{1b}]8;;https://example.test/signup\u{1b}\\\
             \u{1b}[4mhttps://example.test/signup\u{1b}[0m\
             \u{1b}]8;;\u{1b}\\"
        );
        assert!(terminal_hyperlink_sequence("https://example.test/\u{1b}]52", true).is_none());
    }
}

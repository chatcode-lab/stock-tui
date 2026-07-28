use std::{
    fmt,
    io::{self, Stdout, Write, stdout},
    panic,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use crossterm::{
    Command,
    cursor::Show,
    event::{DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

#[cfg(windows)]
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};

pub type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

pub struct TerminalSession {
    terminal: TuiTerminal,
}

impl TerminalSession {
    pub fn start() -> io::Result<Self> {
        install_panic_restore_hook();
        enable_raw_mode()?;
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
            Ok(Self { terminal })
        })();
        if setup.is_err() {
            let _ = restore_terminal();
        }
        setup
    }

    pub fn terminal_mut(&mut self) -> &mut TuiTerminal {
        &mut self.terminal
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = restore_terminal();
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

fn restore_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        stdout(),
        DisableBracketedPaste,
        DisableFocusChange,
        DisableSgrMouseCapture,
        LeaveAlternateScreen,
        Show
    )
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
    let original = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let _ = restore_terminal();
        original(panic_info);
    }));
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

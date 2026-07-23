use std::{
    fmt,
    io::{self, Stdout, stdout},
    panic,
};

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
}

use std::{
    io::{self, Stdout, stdout},
    panic,
};

use crossterm::{
    cursor::Show,
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

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
                EnableMouseCapture,
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
        DisableMouseCapture,
        LeaveAlternateScreen,
        Show
    )
}

fn install_panic_restore_hook() {
    let original = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let _ = restore_terminal();
        original(panic_info);
    }));
}
